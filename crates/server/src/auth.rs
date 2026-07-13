//! Biscuit-based authorization for session join links.
//!
//! The server is the sole token emitter. A token carries `organization`,
//! `workspace`, `session`, `participant`, `role`, and `capability` facts plus a
//! self-expiry check. Verifying a connection forces the token's
//! tenant/workspace/session scope to equal the requested scope, so a token for
//! one boundary can never open another.
//!
//! Identity federation (OIDC / Keycloak) sits in front in TB-4; here we only
//! mint and verify the capability tokens.

use std::time::{Duration, SystemTime};

use biscuit_auth::macros::{authorizer, biscuit, fact};
use biscuit_auth::{Algorithm, AuthorizerLimits, Biscuit, KeyPair, PrivateKey, PublicKey};
use presto_core::RoleAssignment;

use crate::session_identity::{SessionRole, SessionScope, role_assignment_for_actor};

const AUTHORIZER_MAX_FACTS: u64 = 256;
const AUTHORIZER_MAX_ITERATIONS: u64 = 32;
const AUTHORIZER_MAX_TIME: Duration = Duration::from_millis(50);

fn authorizer_limits() -> AuthorizerLimits {
    AuthorizerLimits {
        max_facts: AUTHORIZER_MAX_FACTS,
        max_iterations: AUTHORIZER_MAX_ITERATIONS,
        max_time: AUTHORIZER_MAX_TIME,
    }
}

/// What a token-holder may do in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Host,
    Participant,
}

impl Capability {
    fn as_str(self) -> &'static str {
        match self {
            Capability::Host => "host_minting",
            Capability::Participant => "answer_submit",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "host_minting" | "host" => Some(Capability::Host),
            "answer_submit" | "participant" => Some(Capability::Participant),
            _ => None,
        }
    }

    fn role(self) -> SessionRole {
        match self {
            Capability::Host => SessionRole::Host,
            Capability::Participant => SessionRole::Participant,
        }
    }

    fn legacy_as_str(self) -> &'static str {
        match self {
            Capability::Host => "host",
            Capability::Participant => "participant",
        }
    }

    pub fn is_host(self) -> bool {
        matches!(self, Capability::Host)
    }
}

/// The verified claims extracted from a connect token.
#[derive(Debug, Clone)]
pub struct Claims {
    pub tenant_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub capability: Capability,
}

impl Claims {
    pub fn scope(&self) -> SessionScope {
        SessionScope {
            tenant_id: self.tenant_id.clone(),
            workspace_id: self.workspace_id.clone(),
            session_id: self.session_id.clone(),
        }
    }

    pub fn role_assignment(&self) -> RoleAssignment {
        role_assignment_for_actor(
            &self.scope(),
            self.participant_id.clone(),
            self.capability.role(),
        )
    }
}

/// Verified claims from a guest join-link token: scope only, no participant
/// identity or PII.
#[derive(Debug, Clone)]
pub struct JoinLinkClaims {
    pub tenant_id: String,
    pub workspace_id: String,
    pub session_id: String,
}

impl JoinLinkClaims {
    pub fn scope(&self) -> SessionScope {
        SessionScope {
            tenant_id: self.tenant_id.clone(),
            workspace_id: self.workspace_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
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
    /// starting at `now`. The open wedge derives a deterministic tenant/workspace
    /// scope from the session id so every token carries the mandatory
    /// workspace-identity.v0.1 facts.
    pub fn mint(
        &self,
        session_id: &str,
        participant_id: &str,
        capability: Capability,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<String, AuthError> {
        let scope = SessionScope::for_session(session_id);
        self.mint_scoped(&scope, participant_id, capability, ttl, now)
    }

    /// Mint a short-lived join-link token with no participant identity or PII.
    pub fn mint_join_link(
        &self,
        scope: &SessionScope,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<String, AuthError> {
        scope.validate().map_err(AuthError)?;
        let expiration = now + ttl;
        let builder = biscuit!(
            r#"
            organization({tenant_id});
            workspace({workspace_id});
            session({session_id});
            actor("guest-link", "guest_link");
            role("guest_link");
            capability("participant_join");
            check if time($t), $t < {expiration};
            "#,
            tenant_id = scope.tenant_id.as_str(),
            workspace_id = scope.workspace_id.as_str(),
            session_id = scope.session_id.as_str(),
            expiration = expiration,
        );
        builder
            .build(&self.keypair)
            .map_err(|e| AuthError(format!("build: {e}")))?
            .to_base64()
            .map_err(|e| AuthError(format!("encode: {e}")))
    }

    /// Mint a join token for an explicit tenant/workspace/session scope.
    pub fn mint_scoped(
        &self,
        scope: &SessionScope,
        participant_id: &str,
        capability: Capability,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<String, AuthError> {
        scope.validate().map_err(AuthError)?;
        let expiration = now + ttl;
        let cap = capability.as_str();
        let role = capability.role().as_str();
        let builder = biscuit!(
            r#"
            organization({tenant_id});
            workspace({workspace_id});
            session({session_id});
            actor({participant_id}, "human");
            participant({participant_id});
            role({role});
            capability({cap});
            check if time($t), $t < {expiration};
            "#,
            tenant_id = scope.tenant_id.as_str(),
            workspace_id = scope.workspace_id.as_str(),
            session_id = scope.session_id.as_str(),
            participant_id = participant_id,
            role = role,
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
        let scope = SessionScope::for_session(session_id);
        self.verify_scoped(token_b64, &scope, now)
    }

    /// Verify a guest join-link token against the requested session scope.
    pub fn verify_join_link(
        &self,
        token_b64: &str,
        scope: &SessionScope,
        now: SystemTime,
    ) -> Result<JoinLinkClaims, AuthError> {
        scope.validate().map_err(AuthError)?;
        let token = Biscuit::from_base64(token_b64, self.keypair.public())
            .map_err(|e| AuthError(format!("decode: {e}")))?;
        let mut authorizer = authorizer!(
            r#"
            time({now});
            requested_organization({tenant_id});
            requested_workspace({workspace_id});
            requested_session({session_id});
            operation("participant_join");
            allow if operation("participant_join"), capability("participant_join"), role("guest_link"), actor("guest-link", "guest_link"), organization($o), requested_organization($o), workspace($w), requested_workspace($w), session($s), requested_session($s);
            deny if true;
            "#,
            now = now,
            tenant_id = scope.tenant_id.as_str(),
            workspace_id = scope.workspace_id.as_str(),
            session_id = scope.session_id.as_str(),
        )
        .set_limits(authorizer_limits())
        .build(&token)
        .map_err(|e| AuthError(format!("build: {e}")))?;
        authorizer
            .authorize()
            .map_err(|e| AuthError(format!("denied: {e}")))?;
        let (tenant_id, workspace_id, token_session_id): (String, String, String) = authorizer
            .query_exactly_one("data($o, $w, $s) <- organization($o), workspace($w), session($s)")
            .map_err(|e| AuthError(format!("claims: {e}")))?;
        Ok(JoinLinkClaims {
            tenant_id,
            workspace_id,
            session_id: token_session_id,
        })
    }

    /// Verify a token against an explicit tenant/workspace/session scope.
    pub fn verify_scoped(
        &self,
        token_b64: &str,
        scope: &SessionScope,
        now: SystemTime,
    ) -> Result<Claims, AuthError> {
        scope.validate().map_err(AuthError)?;
        let token = Biscuit::from_base64(token_b64, self.keypair.public())
            .map_err(|e| AuthError(format!("decode: {e}")))?;
        let cap_host = Capability::Host.as_str();
        let cap_participant = Capability::Participant.as_str();
        let legacy_host = Capability::Host.legacy_as_str();
        let legacy_participant = Capability::Participant.legacy_as_str();

        let mut authorizer = authorizer!(
            r#"
            time({now});
            requested_organization({tenant_id});
            requested_workspace({workspace_id});
            requested_session({session_id});
            operation("connect");
            allow if capability({cap_host}), role("host"), actor($p, "human"), participant($p), organization($o), requested_organization($o), workspace($w), requested_workspace($w), session($s), requested_session($s);
            allow if capability({cap_participant}), role("participant"), actor($p, "human"), participant($p), organization($o), requested_organization($o), workspace($w), requested_workspace($w), session($s), requested_session($s);
            allow if capability({legacy_host}), role("host"), actor($p, "human"), participant($p), organization($o), requested_organization($o), workspace($w), requested_workspace($w), session($s), requested_session($s);
            allow if capability({legacy_participant}), role("participant"), actor($p, "human"), participant($p), organization($o), requested_organization($o), workspace($w), requested_workspace($w), session($s), requested_session($s);
            deny if true;
            "#,
            now = now,
            tenant_id = scope.tenant_id.as_str(),
            workspace_id = scope.workspace_id.as_str(),
            session_id = scope.session_id.as_str(),
            cap_host = cap_host,
            cap_participant = cap_participant,
            legacy_host = legacy_host,
            legacy_participant = legacy_participant,
        )
        .set_limits(authorizer_limits())
        .build(&token)
        .map_err(|e| AuthError(format!("build: {e}")))?;
        authorizer
            .authorize()
            .map_err(|e| AuthError(format!("denied: {e}")))?;

        let (tenant_id, workspace_id, token_session_id, participant_id, role, cap): (
            String,
            String,
            String,
            String,
            String,
            String,
        ) = authorizer
            .query_exactly_one(
                "data($o, $w, $s, $p, $r, $c) <- organization($o), workspace($w), session($s), participant($p), role($r), capability($c)",
            )
            .map_err(|e| AuthError(format!("claims: {e}")))?;
        let capability =
            Capability::parse(&cap).ok_or_else(|| AuthError("unknown capability".into()))?;
        if capability.role().as_str() != role {
            return Err(AuthError("role/capability mismatch".into()));
        }

        Ok(Claims {
            tenant_id,
            workspace_id,
            session_id: token_session_id,
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
        .set_limits(authorizer_limits())
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
        assert_eq!(claims.tenant_id, "tenant_local");
        assert_eq!(claims.workspace_id, "workspace_s1");
        assert_eq!(claims.session_id, "s1");
        assert_eq!(claims.participant_id, "host-1");
        assert_eq!(claims.capability, Capability::Host);
        assert!(claims.capability.is_host());
        assert_eq!(claims.role_assignment().role, "host");
        assert!(claims.role_assignment().validate().is_ok());

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
        assert_eq!(pclaims.workspace_id, "workspace_s1");
        assert_eq!(pclaims.capability, Capability::Participant);
        assert_eq!(pclaims.role_assignment().role, "participant");
        assert!(!pclaims.capability.is_host());
    }

    #[test]
    fn join_link_token_roundtrips_scope_and_rejects_connect_verification() {
        let auth = Auth::generate();
        let t = now();
        let scope = SessionScope::try_new("tenant-A", "workspace-A", "s1").unwrap();
        let token = auth
            .mint_join_link(&scope, Duration::from_secs(1800), t)
            .unwrap();
        let claims = auth.verify_join_link(&token, &scope, t).unwrap();
        assert_eq!(claims.tenant_id, "tenant-A");
        assert_eq!(claims.workspace_id, "workspace-A");
        assert_eq!(claims.session_id, "s1");
        let later = t + Duration::from_secs(3600);
        assert!(auth.verify_join_link(&token, &scope, later).is_err());
        assert!(auth.verify(&token, "s1", t).is_err());
    }

    #[test]
    fn join_link_token_rejects_cross_session_tenant_and_workspace() {
        let auth = Auth::generate();
        let t = now();
        let scope = SessionScope::try_new("tenant-A", "workspace-A", "s1").unwrap();
        let token = auth
            .mint_join_link(&scope, Duration::from_secs(1800), t)
            .unwrap();

        let wrong_session = SessionScope::try_new("tenant-A", "workspace-A", "s2").unwrap();
        assert!(auth.verify_join_link(&token, &wrong_session, t).is_err());

        let wrong_tenant = SessionScope::try_new("tenant-B", "workspace-A", "s1").unwrap();
        assert!(auth.verify_join_link(&token, &wrong_tenant, t).is_err());

        let wrong_workspace = SessionScope::try_new("tenant-A", "workspace-B", "s1").unwrap();
        assert!(auth.verify_join_link(&token, &wrong_workspace, t).is_err());
    }

    #[test]
    fn host_and_participant_tokens_are_rejected_by_join_link_verification() {
        let auth = Auth::generate();
        let t = now();
        let scope = SessionScope::try_new("tenant-A", "workspace-A", "s1").unwrap();
        let host = auth
            .mint_scoped(
                &scope,
                "host-1",
                Capability::Host,
                Duration::from_secs(3600),
                t,
            )
            .unwrap();
        let participant = auth
            .mint_scoped(
                &scope,
                "p1",
                Capability::Participant,
                Duration::from_secs(3600),
                t,
            )
            .unwrap();
        assert!(auth.verify_join_link(&host, &scope, t).is_err());
        assert!(auth.verify_join_link(&participant, &scope, t).is_err());
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
    fn scoped_token_cannot_cross_tenant_or_workspace() {
        let auth = Auth::generate();
        let t = now();
        let scope = SessionScope::try_new("tenant-A", "workspace-A", "s1").unwrap();
        let token = auth
            .mint_scoped(
                &scope,
                "p1",
                Capability::Participant,
                Duration::from_secs(3600),
                t,
            )
            .unwrap();
        assert!(auth.verify_scoped(&token, &scope, t).is_ok());

        let wrong_tenant = SessionScope::try_new("tenant-B", "workspace-A", "s1").unwrap();
        assert!(auth.verify_scoped(&token, &wrong_tenant, t).is_err());

        let wrong_workspace = SessionScope::try_new("tenant-A", "workspace-B", "s1").unwrap();
        assert!(auth.verify_scoped(&token, &wrong_workspace, t).is_err());
    }

    #[test]
    fn role_capability_mismatch_is_rejected() {
        let auth = Auth::generate();
        let t = now();
        let scope = SessionScope::try_new("tenant-A", "workspace-A", "s1").unwrap();
        let expiration = t + Duration::from_secs(3600);
        let token = biscuit!(
            r#"
            organization({tenant_id});
            workspace({workspace_id});
            session({session_id});
            actor("p1", "human");
            participant("p1");
            role("participant");
            capability("host_minting");
            check if time($t), $t < {expiration};
            "#,
            tenant_id = scope.tenant_id.as_str(),
            workspace_id = scope.workspace_id.as_str(),
            session_id = scope.session_id.as_str(),
            expiration = expiration,
        )
        .build(&auth.keypair)
        .unwrap()
        .to_base64()
        .unwrap();
        assert!(auth.verify_scoped(&token, &scope, t).is_err());
    }

    #[test]
    fn non_human_session_actor_is_rejected() {
        let auth = Auth::generate();
        let t = now();
        let scope = SessionScope::try_new("tenant-A", "workspace-A", "s1").unwrap();
        let expiration = t + Duration::from_secs(3600);
        let token = biscuit!(
            r#"
            organization({tenant_id});
            workspace({workspace_id});
            session({session_id});
            actor("svc1", "service");
            participant("svc1");
            role("host");
            capability("host_minting");
            check if time($t), $t < {expiration};
            "#,
            tenant_id = scope.tenant_id.as_str(),
            workspace_id = scope.workspace_id.as_str(),
            session_id = scope.session_id.as_str(),
            expiration = expiration,
        )
        .build(&auth.keypair)
        .unwrap()
        .to_base64()
        .unwrap();
        assert!(auth.verify_scoped(&token, &scope, t).is_err());
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
        for _ in 0..64 {
            let claims = b.verify(&token, "s1", t).unwrap();
            assert_eq!(claims.participant_id, "p1");
            assert_eq!(claims.capability, Capability::Participant);
        }
    }

    #[test]
    fn authorizer_limits_are_explicit_and_bounded() {
        let limits = authorizer_limits();
        assert_eq!(limits.max_facts, 256);
        assert_eq!(limits.max_iterations, 32);
        assert_eq!(limits.max_time, Duration::from_millis(50));
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

        // The live session token path stays compatible with the default mint/verify API.
        let session = auth
            .mint("s1", "host", Capability::Host, Duration::from_secs(600), t)
            .unwrap();
        assert!(auth.verify(&session, "s1", t).is_ok());
    }
}
