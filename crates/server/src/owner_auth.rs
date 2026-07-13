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
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::auth::Auth;
use crate::membership::{InMemoryMembershipStore, MembershipStore, PersonalSpace, Role};
use crate::oidc::{OidcClient, OidcConfig, pkce_challenge};
use crate::ratelimit::TokenBucket;

pub const SESSION_COOKIE_NAME: &str = "__Host-rumble_session";
pub const LOGIN_COOKIE_NAME: &str = "__Host-rumble_login";
const LOGIN_TTL: Duration = Duration::from_secs(5 * 60);
const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PENDING_LOGINS: usize = 1024;
// A full bucket followed by five minutes of sustained refill admits at most
// 332 abandoned attempts, well below MAX_PENDING_LOGINS. This global bound is
// independent of untrusted forwarding headers and runs before random allocation.
const LOGIN_RATE_BURST: f64 = 32.0;
const LOGIN_RATE_PER_SEC: f64 = 1.0;
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
    login_cookie_digest: [u8; 32],
    expires_at: Instant,
}

struct LoginStart {
    authorization_url: Url,
    login_cookie: String,
}

#[derive(Clone)]
struct WebSession {
    subject: String,
    user: CurrentUser,
    space: PersonalSpace,
    space_max_confidentiality: ConfidentialityLevel,
    effective_clearance: ConfidentialityLevel,
    biscuit: String,
    expires_at: Instant,
}

#[derive(Clone)]
pub struct AuthenticatedOwner {
    pub user: CurrentUser,
    pub space: CurrentSpace,
    /// Server-computed min(organization ceiling, explicit space grant).
    pub effective_clearance: ConfidentialityLevel,
    pub(crate) subject: String,
}

/// Process-local state adapter. Raw state and session identifiers are never map
/// keys: only SHA-256 digests are retained. Logout removes the digest, providing
/// immediate revocation on this instance.
pub struct OwnerAuth {
    provider: Option<Arc<OidcClient>>,
    membership: Arc<dyn MembershipStore>,
    capability_authority: Arc<Auth>,
    public_origin: Option<String>,
    login_admission: TokenBucket,
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
            login_admission: TokenBucket::new(LOGIN_RATE_BURST, LOGIN_RATE_PER_SEC),
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
            login_admission: TokenBucket::new(LOGIN_RATE_BURST, LOGIN_RATE_PER_SEC),
            pending_logins: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.provider.is_some()
    }

    fn begin_login(&self, headers: &HeaderMap, now: Instant) -> Result<LoginStart, OwnerAuthError> {
        let provider = self.provider.as_ref().ok_or(OwnerAuthError::Unavailable)?;

        // Global admission is deliberately independent of client IP. In
        // particular, X-Forwarded-For is ignored because no trusted-proxy
        // boundary is configured here. Check this before allocating secrets.
        if !self.login_admission.allow_at(now) {
            return Err(OwnerAuthError::Capacity);
        }

        let existing_login_cookie = login_cookie_value(headers);
        let previous_cookie_digest = existing_login_cookie.as_deref().map(digest);
        let state = random_value::<32>()?;
        let nonce = random_value::<32>()?;
        let pkce_verifier = random_value::<32>()?;
        // Reuse a valid browser cookie so repeated login starts replace, rather
        // than accumulate beside, that browser's pending transaction.
        let login_cookie = match existing_login_cookie {
            Some(value) => value,
            None => random_value::<32>()?,
        };
        let login_cookie_digest = digest(&login_cookie);
        let mut attempts = self.pending_logins.lock();
        attempts.retain(|_, attempt| attempt.expires_at > now);
        if let Some(previous_cookie_digest) = previous_cookie_digest {
            attempts.retain(|_, attempt| {
                !bool::from(attempt.login_cookie_digest.ct_eq(&previous_cookie_digest))
            });
        }
        if attempts.len() >= MAX_PENDING_LOGINS {
            return Err(OwnerAuthError::Capacity);
        }
        attempts.insert(
            digest(&state),
            LoginAttempt {
                nonce: nonce.clone(),
                pkce_verifier: pkce_verifier.clone(),
                login_cookie_digest,
                expires_at: now + LOGIN_TTL,
            },
        );
        Ok(LoginStart {
            authorization_url: provider.authorization_url(
                &state,
                &nonce,
                &pkce_challenge(&pkce_verifier),
            ),
            login_cookie,
        })
    }

    fn consume_login(&self, state: &str) -> Option<LoginAttempt> {
        if state.is_empty() || state.len() > 512 {
            return None;
        }
        self.pending_logins.lock().remove(&digest(state))
    }

    async fn finish_login(
        &self,
        headers: &HeaderMap,
        state: &str,
        code: &str,
    ) -> Result<String, OwnerAuthError> {
        // Consume before cookie validation and before any network request. A
        // substituted or failed callback can therefore never be replayed.
        let attempt = self
            .consume_login(state)
            .ok_or(OwnerAuthError::Unauthenticated)?;
        if attempt.expires_at <= Instant::now() {
            return Err(OwnerAuthError::Unauthenticated);
        }
        let presented_cookie =
            login_cookie_value(headers).ok_or(OwnerAuthError::Unauthenticated)?;
        if !bool::from(digest(&presented_cookie).ct_eq(&attempt.login_cookie_digest)) {
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
        // Solo-space provisioning currently grants at most Internal. The IdP
        // ceiling can only reduce that grant; a missing claim was parsed as Public.
        let space_grant = ConfidentialityLevel::Internal;
        let effective_clearance = std::cmp::min(identity.clearance_org, space_grant);
        sessions.insert(
            digest(&session_id),
            WebSession {
                subject: identity.sub,
                user,
                space,
                space_max_confidentiality: space_grant,
                effective_clearance,
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
                    max_confidentiality: session.space_max_confidentiality,
                },
            },
            effective_clearance: session.effective_clearance,
            subject: session.subject,
        })
    }

    /// Capability verification plus a current membership authority recheck.
    /// A valid but revoked session is denied fail-closed.
    pub async fn authenticate_sensitive_headers(
        &self,
        headers: &HeaderMap,
        required_capability: &str,
    ) -> Result<AuthenticatedOwner, OwnerAuthError> {
        let owner = self.authenticate_headers(headers, required_capability)?;
        self.recheck_owner(&owner, required_capability).await?;
        Ok(owner)
    }

    /// Revalidates an already authenticated request immediately before a
    /// security-sensitive side effect or publication.
    pub(crate) async fn recheck_owner(
        &self,
        owner: &AuthenticatedOwner,
        required_capability: &str,
    ) -> Result<(), OwnerAuthError> {
        let role = crate::membership::recheck_sensitive(
            self.membership.as_ref(),
            &owner.space.space.id,
            &owner.subject,
        )
        .await
        .map_err(|_| OwnerAuthError::Unauthenticated)?;
        if required_capability == "add_document" && role < Role::Contributor {
            return Err(OwnerAuthError::Unauthenticated);
        }
        Ok(())
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
    pub(crate) fn test_session(
        capability_authority: Arc<Auth>,
        public_origin: &str,
        space_id: &str,
        effective_clearance: ConfidentialityLevel,
        capabilities: &[SpaceCapability],
    ) -> (Self, String) {
        let subject = "test-owner-subject".to_string();
        let cap_names: Vec<&str> = capabilities
            .iter()
            .map(|capability| match capability {
                SpaceCapability::Read => "read",
                SpaceCapability::Contribute => "contribute",
                SpaceCapability::AddDocument => "add_document",
                SpaceCapability::Invite => "invite",
                SpaceCapability::ManageMembers => "manage_members",
                SpaceCapability::DeleteSpace => "delete_space",
            })
            .collect();
        let biscuit = capability_authority
            .mint_space_token(
                space_id,
                &subject,
                &cap_names,
                SESSION_TTL,
                SystemTime::now(),
            )
            .expect("test capability must mint");
        let session_id = "test_owner_session".to_string();
        let space = PersonalSpace {
            id: space_id.to_string(),
            name: "Test notebook".to_string(),
        };
        let user = CurrentUser {
            actor_id: "actor_test".to_string(),
            display_name: None,
            personal_space_id: space_id.to_string(),
        };
        let membership = Arc::new(InMemoryMembershipStore::new());
        membership.insert_test_owner(space_id, &subject);
        let mut sessions = HashMap::new();
        sessions.insert(
            digest(&session_id),
            WebSession {
                subject,
                user,
                space,
                space_max_confidentiality: ConfidentialityLevel::Internal,
                effective_clearance,
                biscuit,
                expires_at: Instant::now() + SESSION_TTL,
            },
        );
        (
            Self {
                provider: None,
                membership,
                capability_authority,
                public_origin: Some(public_origin.to_string()),
                login_admission: TokenBucket::new(LOGIN_RATE_BURST, LOGIN_RATE_PER_SEC),
                pending_logins: Mutex::new(HashMap::new()),
                sessions: Mutex::new(sessions),
            },
            format!("{SESSION_COOKIE_NAME}={session_id}"),
        )
    }

    #[cfg(test)]
    pub(crate) async fn revoke_test_owner(&self, space_id: &str) {
        self.membership
            .revoke_member(space_id, "test-owner-subject")
            .await
            .unwrap();
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

pub(crate) async fn login(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.owner_auth.begin_login(&headers, Instant::now()) {
        Ok(start) => {
            let mut response = no_store_redirect(start.authorization_url.as_str());
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&login_cookie(&start.login_cookie))
                    .expect("base64url cookie value is a valid header"),
            );
            response
        }
        Err(error) => error_response(error),
    }
}

pub(crate) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<CallbackQuery>, QueryRejection>,
) -> Response {
    let response = match query {
        Err(_) => error_response(OwnerAuthError::InvalidRequest),
        Ok(Query(query)) if query.error.is_some() => {
            if let Some(callback_state) = query.state.as_deref() {
                state.owner_auth.consume_login(callback_state);
            }
            error_response(OwnerAuthError::Unauthenticated)
        }
        Ok(Query(query)) => {
            let (Some(code), Some(callback_state)) = (query.code, query.state) else {
                return expire_login_cookie(error_response(OwnerAuthError::InvalidRequest));
            };
            match state
                .owner_auth
                .finish_login(&headers, &callback_state, &code)
                .await
            {
                Ok(session_id) => {
                    let mut response = no_store_redirect("/app");
                    response.headers_mut().append(
                        header::SET_COOKIE,
                        HeaderValue::from_str(&session_cookie(&session_id))
                            .expect("base64url cookie value is a valid header"),
                    );
                    response
                }
                Err(error) => error_response(error),
            }
        }
    };
    expire_login_cookie(response)
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

fn login_cookie(value: &str) -> String {
    format!(
        "{LOGIN_COOKIE_NAME}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={}",
        LOGIN_TTL.as_secs()
    )
}

fn expire_login_cookie(mut response: Response) -> Response {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "__Host-rumble_login=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0",
        ),
    );
    response
}

fn session_cookie_value(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, SESSION_COOKIE_NAME)
}

fn login_cookie_value(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, LOGIN_COOKIE_NAME)
}

fn cookie_value(headers: &HeaderMap, expected_name: &str) -> Option<String> {
    let mut found = None;
    for header_value in headers.get_all(header::COOKIE) {
        let value = header_value.to_str().ok()?;
        for pair in value.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name == expected_name {
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

    use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der};
    use aws_lc_rs::rsa::{KeyPair as RsaKeyPair, KeySize};
    use aws_lc_rs::signature::KeyPair as _;
    use axum::Form;
    use axum::body::{Body, to_bytes};
    use axum::routing::{get, post};
    use base64::engine::general_purpose::STANDARD;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, get_current_timestamp};
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
        let private = RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
        let public = private.public_key();
        let der = AsDer::<Pkcs8V1Der>::as_der(&private).unwrap();
        let encoded_der = STANDARD.encode(der.as_ref());
        let pem_body = encoded_der
            .as_bytes()
            .chunks(64)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{pem_body}\n-----END PRIVATE KEY-----\n");
        let idp = FakeIdp {
            issuer,
            encoding_key: Arc::new(EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap()),
            modulus: URL_SAFE_NO_PAD.encode(public.modulus().big_endian_without_leading_zero()),
            exponent: URL_SAFE_NO_PAD.encode(public.exponent().big_endian_without_leading_zero()),
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

    struct BrowserLogin {
        state: String,
        cookie: String,
    }

    async fn begin_login(router: &axum::Router, idp: &FakeIdp) -> BrowserLogin {
        begin_login_with_cookie(router, idp, None).await
    }

    async fn begin_login_with_cookie(
        router: &axum::Router,
        idp: &FakeIdp,
        cookie: Option<&str>,
    ) -> BrowserLogin {
        let mut request = axum::http::Request::builder().uri("/auth/login");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let set_cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(set_cookie.starts_with("__Host-rumble_login="));
        assert!(set_cookie.contains("; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=300"));
        BrowserLogin {
            state: idp.configure_login(response.headers()[header::LOCATION].to_str().unwrap()),
            cookie: set_cookie.split(';').next().unwrap().to_string(),
        }
    }

    async fn callback_response(router: &axum::Router, login: &BrowserLogin) -> Response {
        callback_response_with_cookie(router, &login.state, Some(&login.cookie)).await
    }

    async fn callback_response_with_cookie(
        router: &axum::Router,
        state: &str,
        cookie: Option<&str>,
    ) -> Response {
        let mut request = axum::http::Request::builder()
            .uri(format!("/auth/callback?code=one-time-code&state={state}"));
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
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
    fn cookies_have_exact_host_prefix_security_attributes_and_no_domain() {
        let session = session_cookie("opaque_value");
        assert_eq!(
            session,
            "__Host-rumble_session=opaque_value; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=900"
        );
        let login = login_cookie("opaque_value");
        assert_eq!(
            login,
            "__Host-rumble_login=opaque_value; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=300"
        );
        assert!(!session.to_ascii_lowercase().contains("domain="));
        assert!(!login.to_ascii_lowercase().contains("domain="));
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
            login_admission: TokenBucket::new(LOGIN_RATE_BURST, LOGIN_RATE_PER_SEC),
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
        let (router, owner_auth, membership, idp) = configured_app().await;
        assert_eq!(idp.jwks_requests.load(Ordering::SeqCst), 1);
        let login = begin_login(&router, &idp).await;
        assert_eq!(
            callback_response_with_cookie(&router, "attacker-state", Some(&login.cookie))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 0);
        let callback = callback_response(&router, &login).await;
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        assert_eq!(callback.headers()[header::LOCATION], "/app");
        let set_cookie = callback.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(set_cookie.starts_with("__Host-rumble_session="));
        assert!(set_cookie.contains("; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=900"));
        assert!(!set_cookie.to_ascii_lowercase().contains("domain="));
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let mut auth_headers = HeaderMap::new();
        auth_headers.insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(
            owner_auth
                .authenticate_headers(&auth_headers, "read")
                .unwrap()
                .effective_clearance,
            ConfidentialityLevel::Public,
            "missing clearance_org must cap the explicit Internal space grant at Public"
        );
        assert!(
            callback
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|value| value
                    == "__Host-rumble_login=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0")
        );

        let replay = callback_response(&router, &login).await;
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
        let login2 = begin_login(&router, &idp).await;
        assert_eq!(
            callback_response(&router, &login2).await.status(),
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
    async fn callback_is_bound_to_initiating_browser_and_consumed_on_mismatch() {
        let (router, _owner_auth, _membership, idp) = configured_app().await;
        let browser_a = begin_login(&router, &idp).await;
        let browser_b = begin_login(&router, &idp).await;

        let swapped =
            callback_response_with_cookie(&router, &browser_a.state, Some(&browser_b.cookie)).await;
        assert_eq!(swapped.status(), StatusCode::UNAUTHORIZED);
        assert!(
            swapped
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|value| value
                    == "__Host-rumble_login=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0")
        );
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 0);

        // The mismatched callback consumed A's transaction; neither browser can
        // replay it through the server-side PKCE oracle.
        assert_eq!(
            callback_response(&router, &browser_a).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn callback_error_consumes_transaction_and_expires_login_cookie() {
        let (router, _owner_auth, _membership, idp) = configured_app().await;
        let login = begin_login(&router, &idp).await;
        let error = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/auth/callback?error=access_denied&state={}",
                        login.state
                    ))
                    .header(header::COOKIE, &login.cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
        assert!(
            error
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|value| value
                    == "__Host-rumble_login=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0")
        );
        assert_eq!(
            callback_response(&router, &login).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(idp.token_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn one_pending_login_per_browser_cookie_replaces_the_previous_attempt() {
        let (router, owner_auth, _membership, idp) = configured_app().await;
        let first = begin_login(&router, &idp).await;
        let second = begin_login_with_cookie(&router, &idp, Some(&first.cookie)).await;
        assert_eq!(second.cookie, first.cookie);
        assert_eq!(owner_auth.pending_logins.lock().len(), 1);
        assert_eq!(
            callback_response(&router, &first).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            callback_response(&router, &second).await.status(),
            StatusCode::SEE_OTHER
        );
    }

    #[tokio::test]
    async fn global_login_admission_is_bounded_before_1024_pending_allocations() {
        let (_router, owner_auth, _membership, _idp) = configured_app().await;
        let now = Instant::now();
        let headers = HeaderMap::new();
        for _ in 0..LOGIN_RATE_BURST as usize {
            owner_auth.begin_login(&headers, now).unwrap();
        }
        assert_eq!(
            owner_auth.begin_login(&headers, now).err(),
            Some(OwnerAuthError::Capacity)
        );
        assert_eq!(owner_auth.pending_logins.lock().len(), 32);

        // The sustained rate is one attempt/second and cannot fill the map
        // during the five-minute transaction TTL.
        assert!(
            owner_auth
                .begin_login(&headers, now + Duration::from_secs(1))
                .is_ok()
        );
        assert_eq!(
            owner_auth
                .begin_login(&headers, now + Duration::from_secs(1))
                .err(),
            Some(OwnerAuthError::Capacity)
        );
    }

    #[tokio::test]
    async fn state_ttl_and_all_production_claim_failures_are_rejected() {
        let (router, owner_auth, _membership, idp) = configured_app().await;
        let expired_login = begin_login(&router, &idp).await;
        owner_auth.expire_login(&expired_login.state);
        assert_eq!(
            callback_response(&router, &expired_login).await.status(),
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
            let login = begin_login(&router, &idp).await;
            assert_eq!(
                callback_response(&router, &login).await.status(),
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
