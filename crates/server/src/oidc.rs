//! OIDC authentication for the owner web surface.
//!
//! OIDC proves an external identity. It never grants a local space capability:
//! after this module validates an ID token, membership remains authoritative and
//! the server mints the local Biscuit capability kept behind an opaque web
//! session. Discovery and JWKS are fetched with redirects disabled. Keys are
//! cached by `kid`; an unknown `kid` causes at most one cooldown-bounded refresh.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header, get_current_timestamp,
};
use parking_lot::RwLock;
use reqwest::{Client, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROVIDER_BODY: usize = 1024 * 1024;
const UNKNOWN_KID_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);
const ID_TOKEN_MAX_AGE: u64 = 10 * 60;
const CLOCK_SKEW: u64 = 30;

/// Why an ID token was rejected. Variants deliberately contain no token or
/// provider detail and are safe to map to one non-verbose authentication error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcError {
    Invalid,
    NonceMismatch,
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => write!(f, "id_token validation failed"),
            Self::NonceMismatch => write!(f, "id_token nonce mismatch"),
        }
    }
}

impl std::error::Error for OidcError {}

/// Non-sensitive protocol/configuration failures. Callers must not add the
/// authorization code, token, nonce, state, or verifier to these errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcProtocolError {
    Configuration,
    Discovery,
    Provider,
    UnknownKey,
    InvalidToken,
}

impl std::fmt::Display for OidcProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Configuration => "OIDC configuration invalid",
            Self::Discovery => "OIDC discovery failed",
            Self::Provider => "OIDC provider request failed",
            Self::UnknownKey => "OIDC signing key unavailable",
            Self::InvalidToken => "OIDC token invalid",
        };
        f.write_str(message)
    }
}

impl std::error::Error for OidcProtocolError {}

/// The validated identity asserted by the external IdP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcIdentity {
    pub sub: String,
    pub clearance_org: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct IdTokenClaims {
    sub: String,
    aud: Audience,
    iat: u64,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    clearance_org: Option<String>,
}

/// Validate a signed ID token using a key selected from the provider JWKS.
/// Signature, issuer, audience, expiry/not-before, issued-at freshness, optional
/// multi-audience `azp`, and nonce are all fail-closed.
pub fn validate_id_token(
    token: &str,
    key: &DecodingKey,
    algorithm: Algorithm,
    iss: &str,
    aud: &str,
    nonce: &str,
) -> Result<OidcIdentity, OidcError> {
    let mut validation = Validation::new(algorithm);
    validation.set_issuer(&[iss]);
    validation.set_audience(&[aud]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.validate_nbf = true;
    validation.leeway = CLOCK_SKEW;

    let data = decode::<IdTokenClaims>(token, key, &validation).map_err(|_| OidcError::Invalid)?;

    let now = get_current_timestamp();
    if data.claims.iat > now.saturating_add(CLOCK_SKEW)
        || now.saturating_sub(data.claims.iat) > ID_TOKEN_MAX_AGE
    {
        return Err(OidcError::Invalid);
    }

    if let Audience::Many(values) = &data.claims.aud
        && (values.len() > 1 || !values.iter().any(|value| value == aud))
        && data.claims.azp.as_deref() != Some(aud)
    {
        return Err(OidcError::Invalid);
    }
    if let Audience::One(value) = &data.claims.aud
        && value != aud
    {
        return Err(OidcError::Invalid);
    }

    if data.claims.nonce.as_deref() != Some(nonce) {
        return Err(OidcError::NonceMismatch);
    }
    if data.claims.sub.is_empty() || data.claims.sub.len() > 512 {
        return Err(OidcError::Invalid);
    }

    Ok(OidcIdentity {
        sub: data.claims.sub,
        clearance_org: data.claims.clearance_org,
    })
}

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
}

impl OidcConfig {
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
        }
    }
}

#[derive(Deserialize)]
struct ProviderMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default)]
    r#use: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

/// Discovered OIDC endpoints and a rotation-aware RS256 JWKS cache.
pub struct OidcClient {
    config: OidcConfig,
    authorization_endpoint: Url,
    token_endpoint: Url,
    jwks_uri: Url,
    http: Client,
    keys: RwLock<HashMap<String, DecodingKey>>,
    refresh_gate: AsyncMutex<()>,
    last_unknown_refresh: Mutex<Option<Instant>>,
}

impl OidcClient {
    /// Discover the provider and prefill its JWKS cache. Startup fails if
    /// metadata is inconsistent or redirects/endpoints escape the configured
    /// issuer origin. HTTP is accepted only for loopback development.
    pub async fn discover(config: OidcConfig) -> Result<Self, OidcProtocolError> {
        let issuer = validate_endpoint(&config.issuer)?;
        let redirect_uri =
            Url::parse(&config.redirect_uri).map_err(|_| OidcProtocolError::Configuration)?;
        if config.client_id.trim().is_empty() || redirect_uri.fragment().is_some() {
            return Err(OidcProtocolError::Configuration);
        }

        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(HTTP_TIMEOUT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|_| OidcProtocolError::Configuration)?;
        let discovery_url = Url::parse(&format!(
            "{}/.well-known/openid-configuration",
            issuer.as_str().trim_end_matches('/')
        ))
        .map_err(|_| OidcProtocolError::Configuration)?;
        let metadata: ProviderMetadata =
            fetch_json(&http, discovery_url, OidcProtocolError::Discovery).await?;
        if metadata.issuer != config.issuer {
            return Err(OidcProtocolError::Discovery);
        }

        let authorization_endpoint =
            same_origin_endpoint(&issuer, &metadata.authorization_endpoint)?;
        let token_endpoint = same_origin_endpoint(&issuer, &metadata.token_endpoint)?;
        let jwks_uri = same_origin_endpoint(&issuer, &metadata.jwks_uri)?;
        let client = Self {
            config,
            authorization_endpoint,
            token_endpoint,
            jwks_uri,
            http,
            keys: RwLock::new(HashMap::new()),
            refresh_gate: AsyncMutex::new(()),
            last_unknown_refresh: Mutex::new(None),
        };
        client.refresh_jwks().await?;
        Ok(client)
    }

    /// Build the provider redirect URL. The state, nonce and verifier remain
    /// owned by the caller's bounded server-side login transaction store.
    pub fn authorization_url(&self, state: &str, nonce: &str, pkce_challenge: &str) -> Url {
        let mut url = self.authorization_endpoint.clone();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("scope", "openid")
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", pkce_challenge)
            .append_pair("code_challenge_method", "S256");
        url
    }

    /// Exchange one authorization code with its PKCE verifier, then validate
    /// the returned ID token through the existing total-validation seam.
    pub async fn exchange_and_validate(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<OidcIdentity, OidcProtocolError> {
        if code.is_empty() || code.len() > 4096 || verifier.len() < 43 || verifier.len() > 128 {
            return Err(OidcProtocolError::InvalidToken);
        }
        let response = self
            .http
            .post(self.token_endpoint.clone())
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", self.config.client_id.as_str()),
                ("redirect_uri", self.config.redirect_uri.as_str()),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|_| OidcProtocolError::Provider)?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| OidcProtocolError::Provider)?;
        if !status.is_success() || bytes.len() > MAX_PROVIDER_BODY {
            return Err(OidcProtocolError::Provider);
        }
        let tokens: TokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| OidcProtocolError::Provider)?;
        if tokens.id_token.len() > 64 * 1024 {
            return Err(OidcProtocolError::InvalidToken);
        }

        let header =
            decode_header(&tokens.id_token).map_err(|_| OidcProtocolError::InvalidToken)?;
        if header.alg != Algorithm::RS256 {
            return Err(OidcProtocolError::InvalidToken);
        }
        let kid = header.kid.ok_or(OidcProtocolError::InvalidToken)?;
        if kid.is_empty() || kid.len() > 256 {
            return Err(OidcProtocolError::InvalidToken);
        }
        let key = self.key_for(&kid).await?;
        validate_id_token(
            &tokens.id_token,
            &key,
            Algorithm::RS256,
            &self.config.issuer,
            &self.config.client_id,
            nonce,
        )
        .map_err(|_| OidcProtocolError::InvalidToken)
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, OidcProtocolError> {
        if let Some(key) = self.keys.read().get(kid).cloned() {
            return Ok(key);
        }

        let _guard = self.refresh_gate.lock().await;
        if let Some(key) = self.keys.read().get(kid).cloned() {
            return Ok(key);
        }
        {
            let mut last = self
                .last_unknown_refresh
                .lock()
                .map_err(|_| OidcProtocolError::Provider)?;
            if last
                .as_ref()
                .is_some_and(|instant| instant.elapsed() < UNKNOWN_KID_REFRESH_COOLDOWN)
            {
                return Err(OidcProtocolError::UnknownKey);
            }
            *last = Some(Instant::now());
        }
        self.refresh_jwks().await?;
        self.keys
            .read()
            .get(kid)
            .cloned()
            .ok_or(OidcProtocolError::UnknownKey)
    }

    async fn refresh_jwks(&self) -> Result<(), OidcProtocolError> {
        let document: JwksDocument = fetch_json(
            &self.http,
            self.jwks_uri.clone(),
            OidcProtocolError::Provider,
        )
        .await?;
        let mut refreshed = HashMap::new();
        for jwk in document.keys {
            if jwk.kty != "RSA"
                || jwk.alg.as_deref().is_some_and(|alg| alg != "RS256")
                || jwk.r#use.as_deref().is_some_and(|usage| usage != "sig")
            {
                continue;
            }
            let (Some(kid), Some(n), Some(e)) = (jwk.kid, jwk.n, jwk.e) else {
                continue;
            };
            if kid.is_empty() || kid.len() > 256 || refreshed.contains_key(&kid) {
                return Err(OidcProtocolError::Provider);
            }
            let key = DecodingKey::from_rsa_components(&n, &e)
                .map_err(|_| OidcProtocolError::Provider)?;
            refreshed.insert(kid, key);
        }
        if refreshed.is_empty() {
            return Err(OidcProtocolError::Provider);
        }
        *self.keys.write() = refreshed;
        Ok(())
    }
}

fn validate_endpoint(raw: &str) -> Result<Url, OidcProtocolError> {
    let url = Url::parse(raw).map_err(|_| OidcProtocolError::Configuration)?;
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if (!secure && !loopback_http) || url.query().is_some() || url.fragment().is_some() {
        return Err(OidcProtocolError::Configuration);
    }
    Ok(url)
}

fn same_origin_endpoint(issuer: &Url, raw: &str) -> Result<Url, OidcProtocolError> {
    let endpoint = validate_endpoint(raw)?;
    if endpoint.scheme() != issuer.scheme()
        || endpoint.host_str() != issuer.host_str()
        || endpoint.port_or_known_default() != issuer.port_or_known_default()
    {
        return Err(OidcProtocolError::Discovery);
    }
    Ok(endpoint)
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(
    http: &Client,
    url: Url,
    failure: OidcProtocolError,
) -> Result<T, OidcProtocolError> {
    let response = http.get(url).send().await.map_err(|_| failure)?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| failure)?;
    if !status.is_success() || bytes.len() > MAX_PROVIDER_BODY {
        return Err(failure);
    }
    serde_json::from_slice(&bytes).map_err(|_| failure)
}

/// RFC 7636 S256 challenge for a high-entropy verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::{Value, json};

    const ISS: &str = "https://idp.example/realms/presto";
    const AUD: &str = "presto-client";
    const NONCE: &str = "nonce-abc123";
    const SECRET: &[u8] = b"unit-test-hmac-secret-0123456789";

    fn sign(secret: &[u8], claims: &Value) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }

    fn valid_claims() -> Value {
        let now = get_current_timestamp();
        json!({
            "sub": "user-1",
            "iss": ISS,
            "aud": AUD,
            "exp": now + 3600,
            "iat": now,
            "nonce": NONCE,
            "clearance_org": "internal",
        })
    }

    fn validate(token: &str) -> Result<OidcIdentity, OidcError> {
        validate_id_token(
            token,
            &DecodingKey::from_secret(SECRET),
            Algorithm::HS256,
            ISS,
            AUD,
            NONCE,
        )
    }

    #[test]
    fn accepts_a_valid_id_token_and_extracts_identity() {
        let id = validate(&sign(SECRET, &valid_claims())).unwrap();
        assert_eq!(id.sub, "user-1");
        assert_eq!(id.clearance_org.as_deref(), Some("internal"));
    }

    #[test]
    fn rejects_bad_signature_issuer_audience_expiry_nonce_and_iat() {
        let forged = sign(b"a-totally-different-secret-key!!", &valid_claims());
        assert_eq!(validate(&forged), Err(OidcError::Invalid));

        for (field, value) in [
            ("iss", json!("https://attacker.example")),
            ("aud", json!("some-other-client")),
            ("exp", json!(get_current_timestamp() - 3600)),
            ("iat", json!(get_current_timestamp() + 3600)),
            ("nbf", json!(get_current_timestamp() + 3600)),
        ] {
            let mut claims = valid_claims();
            claims[field] = value;
            assert_eq!(validate(&sign(SECRET, &claims)), Err(OidcError::Invalid));
        }

        let mut nonce = valid_claims();
        nonce["nonce"] = json!("nonce-from-an-attacker");
        assert_eq!(
            validate(&sign(SECRET, &nonce)),
            Err(OidcError::NonceMismatch)
        );
    }

    #[test]
    fn rejects_missing_or_empty_subject_and_missing_issued_at() {
        let mut empty_subject = valid_claims();
        empty_subject["sub"] = json!("");
        assert_eq!(
            validate(&sign(SECRET, &empty_subject)),
            Err(OidcError::Invalid)
        );

        for field in ["sub", "iat"] {
            let mut claims = valid_claims();
            claims.as_object_mut().unwrap().remove(field);
            assert_eq!(validate(&sign(SECRET, &claims)), Err(OidcError::Invalid));
        }
    }

    #[test]
    fn multiple_audiences_require_matching_authorized_party() {
        let mut claims = valid_claims();
        claims["aud"] = json!([AUD, "other"]);
        assert_eq!(validate(&sign(SECRET, &claims)), Err(OidcError::Invalid));
        claims["azp"] = json!(AUD);
        assert!(validate(&sign(SECRET, &claims)).is_ok());
    }

    #[test]
    fn pkce_is_sha256_base64url_without_padding() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn rejects_non_loopback_cleartext_provider() {
        assert_eq!(
            validate_endpoint("http://idp.example/realms/demo"),
            Err(OidcProtocolError::Configuration)
        );
        assert!(validate_endpoint("http://localhost:8081/realms/demo").is_ok());
    }
}
