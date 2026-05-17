//! The SP-A authorization error taxonomy and its HTTP mapping.
//!
//! Anti-enumeration is the security property: {not a member, does not exist,
//! revoked} collapse into one **indistinguishable 404** — a prober cannot tell a
//! real-but-forbidden space/resource from a nonexistent one, so it cannot
//! enumerate which ones exist. (Per SP-A §B: the response *body* is uniform;
//! perfectly constant timing is not promised, since the membership path does
//! more work than the short-circuit.) Errors never carry the token.

use axum::http::StatusCode;

/// Why an authorization decision failed (SP-A taxonomy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzError {
    /// No valid identity (missing/invalid id_token or session token).
    Unauthenticated,
    /// Authenticated, but not permitted on this target.
    Forbidden,
    /// The target does not exist.
    NotFound,
    /// Membership/capability was revoked.
    Revoked,
    /// The token's self-expiry has passed.
    Expired,
    /// Rate limit exceeded.
    RateLimited,
}

impl AuthzError {
    /// The HTTP status and an opaque body. `Forbidden`/`NotFound`/`Revoked`
    /// deliberately map to an **identical** 404 so a prober cannot distinguish
    /// them (anti-enumeration).
    pub fn response(self) -> (StatusCode, &'static str) {
        match self {
            // Anti-enumeration: these three are indistinguishable.
            AuthzError::Forbidden | AuthzError::NotFound | AuthzError::Revoked => {
                (StatusCode::NOT_FOUND, "not found")
            }
            AuthzError::Unauthenticated | AuthzError::Expired => {
                (StatusCode::UNAUTHORIZED, "unauthenticated")
            }
            AuthzError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "too many requests"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denials_are_indistinguishable_anti_enumeration() {
        // not-a-member, does-not-exist, and revoked must be byte-identical.
        let forbidden = AuthzError::Forbidden.response();
        let not_found = AuthzError::NotFound.response();
        let revoked = AuthzError::Revoked.response();
        assert_eq!(forbidden, not_found);
        assert_eq!(not_found, revoked);
        assert_eq!(forbidden.0, StatusCode::NOT_FOUND);
    }

    #[test]
    fn authn_failures_are_401_and_rate_limit_is_429() {
        assert_eq!(
            AuthzError::Unauthenticated.response().0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(AuthzError::Expired.response().0, StatusCode::UNAUTHORIZED);
        assert_eq!(
            AuthzError::RateLimited.response().0,
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
