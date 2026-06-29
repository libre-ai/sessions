//! OIDC `id_token` validation — SP-A Increment 1 (authentication), the
//! security-critical core of the "OIDC reject" gate.
//!
//! Validates an OIDC `id_token`: signature (against the key + algorithm the IdP
//! advertised in its JWKS, selected by `kid` upstream), issuer, audience,
//! expiry/not-before, and the anti-replay `nonce`; extracts the subject (`sub`)
//! and `clearance_org` (carried into the SP-A Biscuit, consumed by SP-B). Any
//! failure is a rejection — no unauthenticated token reaches a session.
//!
//! The full Authorization Code + PKCE redirect flow and the JWKS fetch/rotation
//! (discovery, `kid` lookup, key caching) are the rest of Increment 1; this is
//! the validation core, proven by the adversarial tests below. Production uses
//! RS256 keys built from the IdP JWKS; the tests exercise the identical
//! algorithm-independent claim + signature-rejection logic with HS256.

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;

/// Why an `id_token` was rejected. Never carries the token itself.
#[derive(Debug, PartialEq, Eq)]
pub enum OidcError {
    /// Signature, issuer, audience, expiry/not-before, or a required claim
    /// failed validation.
    Invalid(String),
    /// The `nonce` did not match the value the server stored for this login
    /// (anti-replay).
    NonceMismatch,
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OidcError::Invalid(_) => write!(f, "id_token validation failed"),
            OidcError::NonceMismatch => write!(f, "id_token nonce mismatch"),
        }
    }
}

impl std::error::Error for OidcError {}

/// The validated identity an `id_token` asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcIdentity {
    pub sub: String,
    pub clearance_org: Option<String>,
}

#[derive(Deserialize)]
struct IdTokenClaims {
    sub: String,
    // iss / aud / exp / nbf are validated by jsonwebtoken via `Validation`.
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    clearance_org: Option<String>,
}

/// Validate an OIDC `id_token` and return the asserted identity.
///
/// `key` + `algorithm` come from the IdP JWKS (by `kid`) in production; `iss` is
/// the expected issuer; `aud` is this client's id; `nonce` is the value the
/// server stored when it began this login (anti-replay). A small clock leeway
/// absorbs skew between the IdP and the server.
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
    // Require the security-relevant claims to be present; `exp`/`nbf` are checked
    // by default. `sub` presence is required so an identity always exists.
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.leeway = 30;

    let data = decode::<IdTokenClaims>(token, key, &validation)
        .map_err(|e| OidcError::Invalid(e.to_string()))?;

    // `nonce` is an OIDC anti-replay claim, not a JWT spec claim — verify it here.
    if data.claims.nonce.as_deref() != Some(nonce) {
        return Err(OidcError::NonceMismatch);
    }

    Ok(OidcIdentity {
        sub: data.claims.sub,
        clearance_org: data.claims.clearance_org,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode, get_current_timestamp};
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
        json!({
            "sub": "user-1",
            "iss": ISS,
            "aud": AUD,
            "exp": get_current_timestamp() + 3600,
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
    fn rejects_a_bad_signature() {
        // Signed by a different key than the validator trusts.
        let forged = sign(b"a-totally-different-secret-key!!", &valid_claims());
        assert!(matches!(validate(&forged), Err(OidcError::Invalid(_))));
    }

    #[test]
    fn rejects_a_wrong_issuer() {
        let mut c = valid_claims();
        c["iss"] = json!("https://attacker.example");
        assert!(matches!(
            validate(&sign(SECRET, &c)),
            Err(OidcError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_a_wrong_audience() {
        let mut c = valid_claims();
        c["aud"] = json!("some-other-client");
        assert!(matches!(
            validate(&sign(SECRET, &c)),
            Err(OidcError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_an_expired_token() {
        let mut c = valid_claims();
        c["exp"] = json!(get_current_timestamp() - 3600);
        assert!(matches!(
            validate(&sign(SECRET, &c)),
            Err(OidcError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_a_nonce_mismatch() {
        let mut c = valid_claims();
        c["nonce"] = json!("nonce-from-an-attacker");
        assert_eq!(validate(&sign(SECRET, &c)), Err(OidcError::NonceMismatch));
    }

    #[test]
    fn rejects_a_missing_subject() {
        let c = json!({
            "iss": ISS,
            "aud": AUD,
            "exp": get_current_timestamp() + 3600,
            "nonce": NONCE,
        });
        assert!(matches!(
            validate(&sign(SECRET, &c)),
            Err(OidcError::Invalid(_))
        ));
    }
}
