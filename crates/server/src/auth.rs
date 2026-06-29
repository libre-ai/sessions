//! Biscuit-based authorization for session join links.
//!
//! The server is the sole token emitter. A token carries `session`,
//! `participant` and `capability` facts plus a self-expiry check. Verifying a
//! connection forces the token's session to equal the URL session (via
//! `requested_session`), so a token for session A can never open session B.
//!
//! Identity federation (OIDC / Keycloak) sits in front in TB-4; here we only
//! mint and verify the capability tokens.

use std::time::{Duration, SystemTime};

use biscuit_auth::macros::{authorizer, biscuit, fact};
use biscuit_auth::{Algorithm, Biscuit, KeyPair, PrivateKey, PublicKey};

/// What a token-holder may do in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Host,
    Participant,
}

impl Capability {
    fn as_str(self) -> &'static str {
        match self {
            Capability::Host => "host",
            Capability::Participant => "participant",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "host" => Some(Capability::Host),
            "participant" => Some(Capability::Participant),
            _ => None,
        }
    }

    pub fn is_host(self) -> bool {
        matches!(self, Capability::Host)
    }
}

/// The verified claims extracted from a join token.
#[derive(Debug, Clone)]
pub struct Claims {
    pub session_id: String,
    pub participant_id: String,
    pub capability: Capability,
}

/// Verified claims from a space-scoped token (SP-A): which space, who, and the
/// atomic capabilities the bearer holds there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceClaims {
    pub space_id: String,
    pub subject: String,
    pub caps: Vec<String>,
}

/// Token mint/verify failure. Never carries the token itself.
#[derive(Debug)]
pub struct AuthError(pub String);

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "auth error: {}", self.0)
    }
}

impl std::error::Error for AuthError {}

/// Holds the Ed25519 keypair. In production the private key comes from
/// `BISCUIT_PRIVATE_KEY`; [`Auth::generate`] is for tests and local runs.
pub struct Auth {
    keypair: KeyPair,
}

impl Auth {
    pub fn generate() -> Self {
        Self {
            keypair: KeyPair::new(),
        }
    }

    /// Load the emitter keypair from a hex-encoded Ed25519 private key. All
    /// instances of a multi-instance deployment MUST share this key, or a token
    /// minted by one instance is rejected by another.
    pub fn from_private_key_hex(hex: &str) -> Result<Self, AuthError> {
        let private = PrivateKey::from_bytes_hex(hex, Algorithm::Ed25519)
            .map_err(|e| AuthError(format!("invalid private key: {e}")))?;
        Ok(Self {
            keypair: KeyPair::from(&private),
        })
    }

    /// The hex-encoded private key (for `keygen` output / `BISCUIT_PRIVATE_KEY`).
    pub fn private_key_hex(&self) -> String {
        self.keypair.private().to_bytes_hex()
    }

    pub fn public_key(&self) -> PublicKey {
        self.keypair.public()
    }

    /// Mint a join token for `participant_id` in `session_id`, valid for `ttl`
    /// starting at `now`. Taking `now` explicitly (rather than sampling
    /// `SystemTime::now()` internally) keeps mint-time and verify-time on the
    /// same clock, which callers — tests in particular — can pin to a fixed
    /// instant for deterministic expiry behaviour.
    pub fn mint(
        &self,
        session_id: &str,
        participant_id: &str,
        capability: Capability,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<String, AuthError> {
        let expiration = now + ttl;
        let cap = capability.as_str();
        let builder = biscuit!(
            r#"
            session({session_id});
            participant({participant_id});
            capability({cap});
            check if time($t), $t < {expiration};
            "#,
            session_id = session_id,
            participant_id = participant_id,
            cap = cap,
            expiration = expiration,
        );
        builder
            .build(&self.keypair)
            .map_err(|e| AuthError(format!("build: {e}")))?
            .to_base64()
            .map_err(|e| AuthError(format!("encode: {e}")))
    }

    /// Verify a token presented to connect to `session_id` at `now`.
    pub fn verify(
        &self,
        token_b64: &str,
        session_id: &str,
        now: SystemTime,
    ) -> Result<Claims, AuthError> {
        let token = Biscuit::from_base64(token_b64, self.keypair.public())
            .map_err(|e| AuthError(format!("decode: {e}")))?;

        let mut authorizer = authorizer!(
            r#"
            time({now});
            requested_session({session_id});
            operation("connect");
            allow if capability("host"), session($s), requested_session($s);
            allow if capability("participant"), session($s), requested_session($s);
            deny if true;
            "#,
            now = now,
            session_id = session_id,
        )
        .build(&token)
        .map_err(|e| AuthError(format!("build: {e}")))?;
        authorizer
            .authorize()
            .map_err(|e| AuthError(format!("denied: {e}")))?;

        let (participant_id, cap): (String, String) = authorizer
            .query_exactly_one("data($p, $c) <- participant($p), capability($c)")
            .map_err(|e| AuthError(format!("claims: {e}")))?;
        let capability =
            Capability::parse(&cap).ok_or_else(|| AuthError("unknown capability".into()))?;

        Ok(Claims {
            session_id: session_id.to_string(),
            participant_id,
            capability,
        })
    }

    /// Mint a space-scoped capability token (SP-A): the bearer may exercise
    /// `caps` in `space_id`, valid for `ttl` from `now`. Verifying it against a
    /// different space is denied (`requested_space`) — exactly as session tokens
    /// isolate sessions. The live session tokens above are unchanged.
    pub fn mint_space_token(
        &self,
        space_id: &str,
        subject: &str,
        caps: &[&str],
        ttl: Duration,
        now: SystemTime,
    ) -> Result<String, AuthError> {
        let expiration = now + ttl;
        let mut builder = biscuit!(
            r#"
            space({space_id});
            subject({subject});
            check if time($t), $t < {expiration};
            "#,
            space_id = space_id,
            subject = subject,
            expiration = expiration,
        );
        for cap in caps {
            builder = builder
                .fact(fact!("capability({cap})", cap = *cap))
                .map_err(|e| AuthError(format!("fact: {e}")))?;
        }
        builder
            .build(&self.keypair)
            .map_err(|e| AuthError(format!("build: {e}")))?
            .to_base64()
            .map_err(|e| AuthError(format!("encode: {e}")))
    }

    /// Verify a space token presented to act on `space_id`, requiring capability
    /// `cap`. A token minted for another space, or one lacking the capability, is
    /// denied — no over-minting, no cross-space action.
    pub fn verify_space_token(
        &self,
        token_b64: &str,
        space_id: &str,
        cap: &str,
        now: SystemTime,
    ) -> Result<SpaceClaims, AuthError> {
        let token = Biscuit::from_base64(token_b64, self.keypair.public())
            .map_err(|e| AuthError(format!("decode: {e}")))?;
        let mut authorizer = authorizer!(
            r#"
            time({now});
            requested_space({space_id});
            allow if space($s), requested_space($s), capability({cap});
            deny if true;
            "#,
            now = now,
            space_id = space_id,
            cap = cap,
        )
        .build(&token)
        .map_err(|e| AuthError(format!("build: {e}")))?;
        authorizer
            .authorize()
            .map_err(|e| AuthError(format!("denied: {e}")))?;
        let (space, subject): (String, String) = authorizer
            .query_exactly_one("data($s, $u) <- space($s), subject($u)")
            .map_err(|e| AuthError(format!("claims: {e}")))?;
        let caps: Vec<(String,)> = authorizer
            .query("data($c) <- capability($c)")
            .map_err(|e| AuthError(format!("caps: {e}")))?;
        Ok(SpaceClaims {
            space_id: space,
            subject,
            caps: caps.into_iter().map(|(c,)| c).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn mint_then_verify_roundtrips_capability_and_pid() {
        // Pin mint-time and verify-time to one instant so the roundtrip never
        // races the clock: expiration = t + 3600s is always strictly after t.
        let auth = Auth::generate();
        let t = now();
        let token = auth
            .mint(
                "s1",
                "host-1",
                Capability::Host,
                Duration::from_secs(3600),
                t,
            )
            .unwrap();
        let claims = auth.verify(&token, "s1", t).unwrap();
        assert_eq!(claims.participant_id, "host-1");
        assert_eq!(claims.capability, Capability::Host);
        assert!(claims.capability.is_host());

        let ptoken = auth
            .mint(
                "s1",
                "p1",
                Capability::Participant,
                Duration::from_secs(3600),
                t,
            )
            .unwrap();
        let pclaims = auth.verify(&ptoken, "s1", t).unwrap();
        assert_eq!(pclaims.capability, Capability::Participant);
        assert!(!pclaims.capability.is_host());
    }

    #[test]
    fn token_for_one_session_cannot_open_another() {
        let auth = Auth::generate();
        let t = now();
        let token = auth
            .mint(
                "s1",
                "p1",
                Capability::Participant,
                Duration::from_secs(3600),
                t,
            )
            .unwrap();
        assert!(auth.verify(&token, "s2", t).is_err());
    }

    #[test]
    fn expired_token_is_rejected() {
        let auth = Auth::generate();
        let t = now();
        let token = auth
            .mint(
                "s1",
                "p1",
                Capability::Participant,
                Duration::from_secs(1),
                t,
            )
            .unwrap();
        let later = t + Duration::from_secs(10);
        assert!(auth.verify(&token, "s1", later).is_err());
    }

    #[test]
    fn token_signed_by_another_key_is_rejected() {
        let issuer = Auth::generate();
        let attacker = Auth::generate();
        let t = now();
        let forged = attacker
            .mint("s1", "p1", Capability::Host, Duration::from_secs(3600), t)
            .unwrap();
        assert!(issuer.verify(&forged, "s1", t).is_err());
    }

    #[test]
    fn shared_key_lets_one_instance_verify_anothers_token() {
        // Two instances loading the SAME hex key must accept each other's tokens.
        let a = Auth::generate();
        let b = Auth::from_private_key_hex(&a.private_key_hex()).unwrap();
        let t = now();
        let token = a
            .mint(
                "s1",
                "p1",
                Capability::Participant,
                Duration::from_secs(60),
                t,
            )
            .unwrap();
        let claims = b.verify(&token, "s1", t).unwrap();
        assert_eq!(claims.participant_id, "p1");
        assert_eq!(claims.capability, Capability::Participant);
    }

    #[test]
    fn invalid_private_key_hex_is_rejected() {
        assert!(Auth::from_private_key_hex("not-a-valid-hex-key").is_err());
    }

    #[test]
    fn space_token_isolates_spaces_and_enforces_caps() {
        let auth = Auth::generate();
        let t = now();
        let token = auth
            .mint_space_token(
                "space-A",
                "user-1",
                &["read", "add_document"],
                Duration::from_secs(3600),
                t,
            )
            .unwrap();

        // Granted capability in the right space → ok, with the claims.
        let claims = auth
            .verify_space_token(&token, "space-A", "read", t)
            .unwrap();
        assert_eq!(claims.space_id, "space-A");
        assert_eq!(claims.subject, "user-1");
        assert!(claims.caps.contains(&"add_document".to_string()));

        // Same token, a DIFFERENT space → denied (cross-space isolation).
        assert!(
            auth.verify_space_token(&token, "space-B", "read", t)
                .is_err(),
            "a token for space A must never act on space B"
        );

        // Right space, but a capability the token does not hold → denied.
        assert!(
            auth.verify_space_token(&token, "space-A", "manage_members", t)
                .is_err(),
            "a token must not exercise a capability it was not granted"
        );

        // The live session tokens are untouched by the space generalization.
        let session = auth
            .mint("s1", "host", Capability::Host, Duration::from_secs(600), t)
            .unwrap();
        assert!(auth.verify(&session, "s1", t).is_ok());
    }
}
