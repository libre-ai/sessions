//! Owner web authentication, opaque sessions and HTTP projections.
//!
//! The browser receives only a random, revocable session identifier. The local
//! Biscuit capability is minted by the server after membership bootstrap and is
//! retained server-side; it is never serialized into a URL, JSON body, browser
//! storage, or cookie.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::Json;
use axum::extract::{Query, State, rejection::QueryRejection};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use parking_lot::Mutex;
use presto_core::api::{
    ApiEnvelope, ConfidentialityLevel, CurrentSpace, CurrentUser, SpaceCapability, SpaceRole,
    SpaceSummary,
};
use reqwest::Url;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::auth::Auth;
use crate::membership::{InMemoryMembershipStore, MembershipStore, PersonalSpace, Role};
use crate::oidc::{OidcClient, OidcConfig, pkce_challenge};

pub const SESSION_COOKIE_NAME: &str = "__Host-rumble_session";
const LOGIN_TTL: Duration = Duration::from_secs(5 * 60);
const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PENDING_LOGINS: usize = 1024;
const MAX_WEB_SESSIONS: usize = 10_000;
const OWNER_CAPS: &[&str] = &[
    "read",
    "contribute",
    "add_document",
    "invite",
    "manage_members",
    "delete_space",
];

#[derive(Debug, Clone)]
pub struct OwnerAuthConfig {
    pub oidc: OidcConfig,
    /// Exact browser origin accepted for cookie-authenticated unsafe requests.
    pub public_origin: String,
}

impl OwnerAuthConfig {
    pub fn new(oidc: OidcConfig) -> Result<Self, OwnerAuthError> {
        let redirect = Url::parse(&oidc.redirect_uri).map_err(|_| OwnerAuthError::Configuration)?;
        let secure = redirect.scheme() == "https";
        let loopback_http = redirect.scheme() == "http"
            && redirect.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
        if (!secure && !loopback_http)
            || !redirect.username().is_empty()
            || redirect.password().is_some()
            || redirect.path() != "/auth/callback"
            || redirect.query().is_some()
            || redirect.fragment().is_some()
        {
            return Err(OwnerAuthError::Configuration);
        }
        let public_origin = redirect.origin().ascii_serialization();
        if public_origin == "null" {
            return Err(OwnerAuthError::Configuration);
        }
        Ok(Self {
            oidc,
            public_origin,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerAuthError {
    Configuration,
    Unavailable,
    InvalidRequest,
    Unauthenticated,
    Capacity,
}

impl std::fmt::Display for OwnerAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Configuration => "owner authentication configuration invalid",
            Self::Unavailable => "owner authentication unavailable",
            Self::InvalidRequest => "authentication request invalid",
            Self::Unauthenticated => "unauthenticated",
            Self::Capacity => "authentication capacity reached",
        };
        f.write_str(message)
    }
}

impl std::error::Error for OwnerAuthError {}

#[derive(Clone)]
struct LoginAttempt {
    nonce: String,
    pkce_verifier: String,
    expires_at: Instant,
}

#[derive(Clone)]
struct WebSession {
    subject: String,
    user: CurrentUser,
    space: PersonalSpace,
    biscuit: String,
    expires_at: Instant,
}

#[derive(Clone)]
pub struct AuthenticatedOwner {
    pub user: CurrentUser,
    pub space: CurrentSpace,
}

/// Process-local state adapter. Raw state and session identifiers are never map
/// keys: only SHA-256 digests are retained. Logout removes the digest, providing
/// immediate revocation on this instance.
pub struct OwnerAuth {
    provider: Option<Arc<OidcClient>>,
    membership: Arc<dyn MembershipStore>,
    capability_authority: Arc<Auth>,
    public_origin: Option<String>,
    pending_logins: Mutex<HashMap<[u8; 32], LoginAttempt>>,
    sessions: Mutex<HashMap<[u8; 32], WebSession>>,
}

impl OwnerAuth {
    pub fn disabled(capability_authority: Arc<Auth>) -> Self {
        Self {
            provider: None,
            membership: Arc::new(InMemoryMembershipStore::new()),
            capability_authority,
            public_origin: None,
            pending_logins: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn discover(
        config: OwnerAuthConfig,
        capability_authority: Arc<Auth>,
        membership: Arc<dyn MembershipStore>,
    ) -> Result<Self, OwnerAuthError> {
        let provider = OidcClient::discover(config.oidc)
            .await
            .map_err(|_| OwnerAuthError::Unavailable)?;
        Ok(Self {
            provider: Some(Arc::new(provider)),
            membership,
            capability_authority,
            public_origin: Some(config.public_origin),
            pending_logins: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.provider.is_some()
    }

    fn begin_login(&self) -> Result<Url, OwnerAuthError> {
        let provider = self.provider.as_ref().ok_or(OwnerAuthError::Unavailable)?;
        let state = random_value::<32>()?;
        let nonce = random_value::<32>()?;
        let pkce_verifier = random_value::<32>()?;
        let now = Instant::now();
        let mut attempts = self.pending_logins.lock();
        attempts.retain(|_, attempt| attempt.expires_at > now);
        if attempts.len() >= MAX_PENDING_LOGINS {
            return Err(OwnerAuthError::Capacity);
        }
        attempts.insert(
            digest(&state),
            LoginAttempt {
                nonce: nonce.clone(),
                pkce_verifier: pkce_verifier.clone(),
                expires_at: now + LOGIN_TTL,
            },
        );
        Ok(provider.authorization_url(&state, &nonce, &pkce_challenge(&pkce_verifier)))
    }

    async fn finish_login(&self, state: &str, code: &str) -> Result<String, OwnerAuthError> {
        if state.is_empty() || state.len() > 512 {
            return Err(OwnerAuthError::Unauthenticated);
        }
        // Consume before any network request. Provider failure therefore cannot
        // turn one callback into a replayable login transaction.
        let attempt = self
            .pending_logins
            .lock()
            .remove(&digest(state))
            .ok_or(OwnerAuthError::Unauthenticated)?;
        if attempt.expires_at <= Instant::now() {
            return Err(OwnerAuthError::Unauthenticated);
        }
        let provider = self.provider.as_ref().ok_or(OwnerAuthError::Unavailable)?;
        let identity = provider
            .exchange_and_validate(code, &attempt.pkce_verifier, &attempt.nonce)
            .await
            .map_err(|_| OwnerAuthError::Unauthenticated)?;

        let space = self
            .membership
            .ensure_personal_space(&identity.sub)
            .await
            .map_err(|_| OwnerAuthError::Unavailable)?;
        let membership = self
            .membership
            .member(&space.id, &identity.sub)
            .await
            .ok_or(OwnerAuthError::Unauthenticated)?;
        if membership.role != Role::Owner {
            return Err(OwnerAuthError::Unauthenticated);
        }

        let now = SystemTime::now();
        let biscuit = self
            .capability_authority
            .mint_space_token(&space.id, &identity.sub, OWNER_CAPS, SESSION_TTL, now)
            .map_err(|_| OwnerAuthError::Unavailable)?;
        let session_id = random_value::<32>()?;
        let user = CurrentUser {
            actor_id: actor_id(&identity.sub),
            display_name: None,
            personal_space_id: space.id.clone(),
        };
        let instant = Instant::now();
        let mut sessions = self.sessions.lock();
        sessions.retain(|_, session| session.expires_at > instant);
        if sessions.len() >= MAX_WEB_SESSIONS {
            return Err(OwnerAuthError::Capacity);
        }
        sessions.insert(
            digest(&session_id),
            WebSession {
                subject: identity.sub,
                user,
                space,
                biscuit,
                expires_at: instant + SESSION_TTL,
            },
        );
        Ok(session_id)
    }

    pub fn authenticate_headers(
        &self,
        headers: &HeaderMap,
        required_capability: &str,
    ) -> Result<AuthenticatedOwner, OwnerAuthError> {
        let session_id = session_cookie_value(headers).ok_or(OwnerAuthError::Unauthenticated)?;
        let key = digest(&session_id);
        let session = {
            let mut sessions = self.sessions.lock();
            let Some(session) = sessions.get(&key).cloned() else {
                return Err(OwnerAuthError::Unauthenticated);
            };
            if session.expires_at <= Instant::now() {
                sessions.remove(&key);
                return Err(OwnerAuthError::Unauthenticated);
            }
            session
        };

        let claims = self
            .capability_authority
            .verify_space_token(
                &session.biscuit,
                &session.space.id,
                required_capability,
                SystemTime::now(),
            )
            .map_err(|_| {
                self.sessions.lock().remove(&key);
                OwnerAuthError::Unauthenticated
            })?;
        if claims.subject != session.subject || claims.space_id != session.space.id {
            self.sessions.lock().remove(&key);
            return Err(OwnerAuthError::Unauthenticated);
        }

        let capabilities = [
            ("read", SpaceCapability::Read),
            ("contribute", SpaceCapability::Contribute),
            ("add_document", SpaceCapability::AddDocument),
            ("invite", SpaceCapability::Invite),
            ("manage_members", SpaceCapability::ManageMembers),
            ("delete_space", SpaceCapability::DeleteSpace),
        ]
        .into_iter()
        .filter_map(|(name, capability)| {
            claims
                .caps
                .iter()
                .any(|cap| cap == name)
                .then_some(capability)
        })
        .collect();

        Ok(AuthenticatedOwner {
            user: session.user,
            space: CurrentSpace {
                space: SpaceSummary {
                    id: session.space.id,
                    name: session.space.name,
                    role: SpaceRole::Owner,
                    capabilities,
                    max_confidentiality: ConfidentialityLevel::Internal,
                },
            },
        })
    }

    fn logout(&self, headers: &HeaderMap) {
        if let Some(session_id) = session_cookie_value(headers) {
            self.sessions.lock().remove(&digest(&session_id));
        }
    }

    /// Defense in depth for every unsafe request carrying the auth cookie. Both
    /// Fetch Metadata and exact `Origin` must agree; `Sec-Fetch-Site` alone is
    /// never treated as authorization.
    pub fn same_origin_cookie_request(&self, headers: &HeaderMap) -> bool {
        let Some(expected_origin) = self.public_origin.as_deref() else {
            return false;
        };
        headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            == Some("same-origin")
            && headers
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                == Some(expected_origin)
    }

    pub fn has_auth_cookie(&self, headers: &HeaderMap) -> bool {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .any(|(name, _)| name == SESSION_COOKIE_NAME)
    }

    #[cfg(test)]
    fn expire_login(&self, state: &str) {
        if let Some(attempt) = self.pending_logins.lock().get_mut(&digest(state)) {
            attempt.expires_at = Instant::now();
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub(crate) async fn login(State(state): State<AppState>) -> Response {
    match state.owner_auth.begin_login() {
        Ok(url) => no_store_redirect(url.as_str()),
        Err(error) => error_response(error),
    }
}

pub(crate) async fn callback(
    State(state): State<AppState>,
    query: Result<Query<CallbackQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return error_response(OwnerAuthError::InvalidRequest);
    };
    if query.error.is_some() {
        return error_response(OwnerAuthError::Unauthenticated);
    }
    let (Some(code), Some(callback_state)) = (query.code, query.state) else {
        return error_response(OwnerAuthError::InvalidRequest);
    };
    match state.owner_auth.finish_login(&callback_state, &code).await {
        Ok(session_id) => {
            let mut response = no_store_redirect("/app");
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&session_cookie(&session_id))
                    .expect("base64url cookie value is a valid header"),
            );
            response
        }
        Err(error) => error_response(error),
    }
}

pub(crate) async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    state.owner_auth.logout(&headers);
    let mut response = no_store_redirect("/app/login");
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "__Host-rumble_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
        ),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.owner_auth.authenticate_headers(&headers, "read") {
        Ok(owner) => no_store_json(ApiEnvelope { data: owner.user }),
        Err(error) => error_response(error),
    }
}

pub(crate) async fn current_space(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.owner_auth.authenticate_headers(&headers, "read") {
        Ok(owner) => no_store_json(ApiEnvelope { data: owner.space }),
        Err(error) => error_response(error),
    }
}

fn no_store_json<T: serde::Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn no_store_redirect(location: &str) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let Ok(location) = HeaderValue::from_str(location) else {
        return error_response(OwnerAuthError::Unavailable);
    };
    response.headers_mut().insert(header::LOCATION, location);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

fn error_response(error: OwnerAuthError) -> Response {
    let (status, body) = match error {
        OwnerAuthError::Configuration | OwnerAuthError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication unavailable",
        ),
        OwnerAuthError::Capacity => (StatusCode::TOO_MANY_REQUESTS, "authentication unavailable"),
        OwnerAuthError::InvalidRequest => {
            (StatusCode::BAD_REQUEST, "invalid authentication request")
        }
        OwnerAuthError::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
    };
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
        .into_response()
}

fn random_value<const N: usize>() -> Result<String, OwnerAuthError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| OwnerAuthError::Unavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn actor_id(subject: &str) -> String {
    let encoded = URL_SAFE_NO_PAD.encode(digest(subject));
    format!("actor_{}", &encoded[..22])
}

fn session_cookie(value: &str) -> String {
    format!(
        "{SESSION_COOKIE_NAME}={value}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
        SESSION_TTL.as_secs()
    )
}

fn session_cookie_value(headers: &HeaderMap) -> Option<String> {
    let mut found = None;
    for header_value in headers.get_all(header::COOKIE) {
        let value = header_value.to_str().ok()?;
        for pair in value.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name == SESSION_COOKIE_NAME {
                if found.is_some()
                    || value.is_empty()
                    || value.len() > 512
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
                {
                    return None;
                }
                found = Some(value.to_string());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use axum::Form;
    use axum::body::{Body, to_bytes};
    use axum::routing::{get, post};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, get_current_timestamp};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::traits::PublicKeyParts;
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{AppState, app};

    #[derive(Clone, Copy)]
    enum TokenMode {
        Valid,
        WrongNonce,
        WrongIssuer,
        WrongAudience,
        Expired,
        BadSignature,
        UnknownKid,
    }

    #[derive(Clone)]
    struct FakeIdp {
        issuer: String,
        encoding_key: Arc<EncodingKey>,
        modulus: String,
        exponent: String,
        expected_nonce: Arc<Mutex<Option<String>>>,
        expected_challenge: Arc<Mutex<Option<String>>>,
        mode: Arc<Mutex<TokenMode>>,
        jwks_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
        fail_jwks: Arc<AtomicBool>,
    }

    impl FakeIdp {
        fn configure_login(&self, location: &str) -> String {
            let url = Url::parse(location).unwrap();
            let values: HashMap<_, _> = url.query_pairs().into_owned().collect();
            *self.expected_nonce.lock() = Some(values["nonce"].clone());
            *self.expected_challenge.lock() = Some(values["code_challenge"].clone());
            assert_eq!(values["code_challenge_method"], "S256");
            values["state"].clone()
        }
    }

    async fn fake_discovery(State(idp): State<FakeIdp>) -> Json<Value> {
        Json(json!({
            "issuer": idp.issuer,
            "authorization_endpoint": format!("{}/authorize", idp.issuer),
            "token_endpoint": format!("{}/token", idp.issuer),
            "jwks_uri": format!("{}/jwks", idp.issuer),
        }))
    }

    async fn fake_jwks(State(idp): State<FakeIdp>) -> Response {
        idp.jwks_requests.fetch_add(1, Ordering::SeqCst);
        if idp.fail_jwks.load(Ordering::SeqCst) {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Json(json!({
            "keys": [{
                "kid": "known-key",
                "kty": "RSA",
                "alg": "RS256",
                "use": "sig",
                "n": idp.modulus,
                "e": idp.exponent,
            }]
        }))
        .into_response()
    }

    async fn fake_token(
        State(idp): State<FakeIdp>,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        idp.token_requests.fetch_add(1, Ordering::SeqCst);
        let expected_challenge = idp.expected_challenge.lock().clone();
        let valid_pkce = form
            .get("code_verifier")
            .map(|verifier| pkce_challenge(verifier))
            == expected_challenge;
        if form.get("code").map(String::as_str) != Some("one-time-code") || !valid_pkce {
            return StatusCode::BAD_REQUEST.into_response();
        }

        let mode = *idp.mode.lock();
        let now = get_current_timestamp();
        let mut claims = json!({
            "sub": "external-subject-1",
            "iss": idp.issuer,
            "aud": "owner-client",
            "exp": now + 300,
            "iat": now,
            "nonce": idp.expected_nonce.lock().clone().unwrap(),
        });
        match mode {
            TokenMode::WrongNonce => claims["nonce"] = json!("wrong-nonce"),
            TokenMode::WrongIssuer => claims["iss"] = json!("http://attacker.invalid"),
            TokenMode::WrongAudience => claims["aud"] = json!("other-client"),
            TokenMode::Expired => claims["exp"] = json!(now - 3600),
            TokenMode::Valid | TokenMode::BadSignature | TokenMode::UnknownKid => {}
        }
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(
            if matches!(mode, TokenMode::UnknownKid) {
                "rotated-but-unavailable"
            } else {
                "known-key"
            }
            .into(),
        );
        let mut id_token = encode(&header, &claims, &idp.encoding_key).unwrap();
        if matches!(mode, TokenMode::BadSignature) {
            let last = id_token.pop().unwrap();
            id_token.push(if last == 'A' { 'B' } else { 'A' });
        }
        Json(json!({ "id_token": id_token, "access_token": "not-used" })).into_response()
    }

    async fn spawn_fake_idp() -> FakeIdp {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let private = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let public = RsaPublicKey::from(&private);
        let der = private.to_pkcs1_der().unwrap();
        let idp = FakeIdp {
            issuer,
            encoding_key: Arc::new(EncodingKey::from_rsa_der(der.as_bytes())),
            modulus: URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            exponent: URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
            expected_nonce: Arc::new(Mutex::new(None)),
            expected_challenge: Arc::new(Mutex::new(None)),
            mode: Arc::new(Mutex::new(TokenMode::Valid)),
            jwks_requests: Arc::new(AtomicUsize::new(0)),
            token_requests: Arc::new(AtomicUsize::new(0)),
            fail_jwks: Arc::new(AtomicBool::new(false)),
        };
        let router = axum::Router::new()
            .route("/.well-known/openid-configuration", get(fake_discovery))
            .route("/jwks", get(fake_jwks))
            .route("/token", post(fake_token))
            .with_state(idp.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        idp
    }

    async fn configured_app() -> (
        axum::Router,
        Arc<OwnerAuth>,
        Arc<InMemoryMembershipStore>,
        FakeIdp,
    ) {
        let idp = spawn_fake_idp().await;
        let authority = Arc::new(Auth::generate());
        let membership = Arc::new(InMemoryMembershipStore::new());
        let config = OwnerAuthConfig::new(OidcConfig::new(
            idp.issuer.clone(),
            "owner-client",
            "http://localhost:3000/auth/callback",
        ))
        .unwrap();
        let owner_auth = Arc::new(
            OwnerAuth::discover(config, authority.clone(), membership.clone())
                .await
                .unwrap(),
        );
        let mut state = AppState::in_memory(authority);
        state.owner_auth = owner_auth.clone();
        (app(state), owner_auth, membership, idp)
    }

    async fn begin_login(router: &axum::Router, idp: &FakeIdp) -> String {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/auth/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        idp.configure_login(response.headers()[header::LOCATION].to_str().unwrap())
    }

    async fn callback_response(router: &axum::Router, state: &str) -> Response {
        router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/auth/callback?code=one-time-code&state={state}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[test]
    fn redirect_uri_requires_https_or_loopback_and_exact_callback_path() {
        assert!(
            OwnerAuthConfig::new(OidcConfig::new(
                "https://idp.example/realms/demo",
                "client",
                "http://app.example/auth/callback",
            ))
            .is_err()
        );
        assert!(
            OwnerAuthConfig::new(OidcConfig::new(
                "https://idp.example/realms/demo",
                "client",
                "https://app.example/other",
            ))
            .is_err()
        );
        assert!(
            OwnerAuthConfig::new(OidcConfig::new(
                "http://localhost:8081/realms/demo",
                "client",
                "http://localhost:3000/auth/callback",
            ))
            .is_ok()
        );
    }

    #[test]
    fn cookie_has_exact_host_prefix_security_attributes_and_no_domain() {
        let cookie = session_cookie("opaque_value");
        assert_eq!(
            cookie,
            "__Host-rumble_session=opaque_value; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=900"
        );
        assert!(!cookie.to_ascii_lowercase().contains("domain="));
    }

    #[test]
    fn cookie_parser_rejects_duplicates_invalid_values_and_non_host_names() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("__Host-rumble_session=valid_opaque-1"),
        );
        assert_eq!(
            session_cookie_value(&headers).as_deref(),
            Some("valid_opaque-1")
        );
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("__Host-rumble_session=one; __Host-rumble_session=two"),
        );
        assert!(session_cookie_value(&headers).is_none());
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("rumble_session=not-host-prefixed"),
        );
        assert!(session_cookie_value(&headers).is_none());
    }

    #[test]
    fn csrf_requires_both_exact_origin_and_fetch_metadata() {
        let auth = OwnerAuth {
            provider: None,
            membership: Arc::new(InMemoryMembershipStore::new()),
            capability_authority: Arc::new(Auth::generate()),
            public_origin: Some("https://app.example".into()),
            pending_logins: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        };
        let mut headers = HeaderMap::new();
        assert!(!auth.same_origin_cookie_request(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(!auth.same_origin_cookie_request(&headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(!auth.same_origin_cookie_request(&headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://app.example"),
        );
        assert!(auth.same_origin_cookie_request(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!auth.same_origin_cookie_request(&headers));
    }

    #[tokio::test]
    async fn full_login_projects_dtos_bootstraps_once_replays_safely_and_logs_out() {
        let (router, _owner_auth, membership, idp) = configured_app().await;
        assert_eq!(idp.jwks_requests.load(Ordering::SeqCst), 1);
        let state = begin_login(&router, &idp).await;
        assert_eq!(
            callback_response(&router, "attacker-state").await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 0);
        let callback = callback_response(&router, &state).await;
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        assert_eq!(callback.headers()[header::LOCATION], "/app");
        let set_cookie = callback.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(set_cookie.starts_with("__Host-rumble_session="));
        assert!(set_cookie.contains("; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=900"));
        assert!(!set_cookie.to_ascii_lowercase().contains("domain="));
        let cookie = set_cookie.split(';').next().unwrap().to_string();

        let replay = callback_response(&router, &state).await;
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 1);

        let me = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/me")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        assert_eq!(me.headers()[header::CACHE_CONTROL], "no-store");
        let me_json: Value =
            serde_json::from_slice(&to_bytes(me.into_body(), 4096).await.unwrap()).unwrap();
        assert!(
            me_json["data"]["actor_id"]
                .as_str()
                .unwrap()
                .starts_with("actor_")
        );
        assert!(me_json["data"].get("display_name").is_none());
        let space_id = me_json["data"]["personal_space_id"]
            .as_str()
            .unwrap()
            .to_string();
        let serialized = serde_json::to_string(&me_json).unwrap();
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("external-subject-1"));

        let current = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/spaces/current")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let current_json: Value =
            serde_json::from_slice(&to_bytes(current.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(current_json["data"]["space"]["id"], space_id);
        assert_eq!(current_json["data"]["space"]["role"], "owner");
        assert_eq!(
            current_json["data"]["space"]["capabilities"]
                .as_array()
                .unwrap()
                .len(),
            6
        );

        // A second complete login is idempotent at the membership authority.
        let state2 = begin_login(&router, &idp).await;
        assert_eq!(
            callback_response(&router, &state2).await.status(),
            StatusCode::SEE_OTHER
        );
        assert_eq!(membership.list_members(&space_id).await.unwrap().len(), 1);
        assert_eq!(
            membership
                .personal_space("external-subject-1")
                .await
                .unwrap()
                .unwrap()
                .id,
            space_id
        );

        for (fetch_site, origin) in [
            (None, None),
            (Some("cross-site"), Some("http://localhost:3000")),
            (Some("invalid"), Some("http://localhost:3000")),
            (Some("same-origin"), Some("http://evil.example")),
        ] {
            let mut request = axum::http::Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header(header::COOKIE, &cookie);
            if let Some(value) = fetch_site {
                request = request.header("sec-fetch-site", value);
            }
            if let Some(value) = origin {
                request = request.header(header::ORIGIN, value);
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }

        let logout = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(header::COOKIE, &cookie)
                    .header("sec-fetch-site", "same-origin")
                    .header(header::ORIGIN, "http://localhost:3000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::SEE_OTHER);
        assert_eq!(logout.headers()[header::LOCATION], "/app/login");
        assert_eq!(
            logout.headers()[header::SET_COOKIE],
            "__Host-rumble_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"
        );
        let after_logout = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/me")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after_logout.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn state_ttl_and_all_production_claim_failures_are_rejected() {
        let (router, owner_auth, _membership, idp) = configured_app().await;
        let expired_state = begin_login(&router, &idp).await;
        owner_auth.expire_login(&expired_state);
        assert_eq!(
            callback_response(&router, &expired_state).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 0);

        for mode in [
            TokenMode::WrongNonce,
            TokenMode::WrongIssuer,
            TokenMode::WrongAudience,
            TokenMode::Expired,
            TokenMode::BadSignature,
        ] {
            *idp.mode.lock() = mode;
            let state = begin_login(&router, &idp).await;
            assert_eq!(
                callback_response(&router, &state).await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
    }

    #[tokio::test]
    async fn unknown_kid_refresh_failure_is_bounded() {
        let (router, _owner_auth, _membership, idp) = configured_app().await;
        *idp.mode.lock() = TokenMode::UnknownKid;
        idp.fail_jwks.store(true, Ordering::SeqCst);

        let first = begin_login(&router, &idp).await;
        assert_eq!(
            callback_response(&router, &first).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(idp.jwks_requests.load(Ordering::SeqCst), 2);

        let second = begin_login(&router, &idp).await;
        assert_eq!(
            callback_response(&router, &second).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            idp.jwks_requests.load(Ordering::SeqCst),
            2,
            "unknown kids inside the cooldown must not amplify JWKS traffic"
        );
    }
}
