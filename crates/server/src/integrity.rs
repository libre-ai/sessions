//! SP-B content integrity: bind each ingested chunk's text to a keyed signature
//! (HMAC-SHA256) so a chunk can be proven to be the one ingested, unaltered.
//!
//! The grounding-verifier credits a question only against source text it can
//! trust; an integrity tag makes tampering (in the store, in transit) detectable
//! — any change to the text or the tag fails verification. The key is the
//! server's ingestion secret (env-provided in production), never logged.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// A hex-encoded HMAC-SHA256 tag binding `content` to `key`. Stored alongside the
/// chunk at ingestion; recomputed and compared before the text is trusted.
pub fn sign_content(key: &[u8], content: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(content.as_bytes());
    to_hex(&mac.finalize().into_bytes())
}

/// Constant-time verification that `tag` is the integrity tag of `content` under
/// `key`. Any alteration of the text, the tag, or the key returns `false`.
pub fn verify_content(key: &[u8], content: &str, tag: &str) -> bool {
    let Some(expected) = from_hex(tag) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(content.as_bytes());
    // `verify_slice` is a constant-time comparison (no early-exit timing leak).
    mac.verify_slice(&expected).is_ok()
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"server-ingestion-secret-key-v1";
    const CONTENT: &str = "The mitochondrion is the powerhouse of the cell.";

    #[test]
    fn a_signed_chunk_verifies() {
        let tag = sign_content(KEY, CONTENT);
        assert!(!tag.is_empty());
        assert!(verify_content(KEY, CONTENT, &tag));
    }

    #[test]
    fn tampered_content_fails_verification() {
        let tag = sign_content(KEY, CONTENT);
        assert!(
            !verify_content(
                KEY,
                "The mitochondrion is the powerhouse of the CELL.",
                &tag
            ),
            "any change to the text must be detected"
        );
    }

    #[test]
    fn tampered_tag_or_wrong_key_fails() {
        let tag = sign_content(KEY, CONTENT);
        let mut forged = tag.clone();
        forged.replace_range(0..1, if tag.starts_with('0') { "1" } else { "0" });
        assert!(
            !verify_content(KEY, CONTENT, &forged),
            "a flipped tag fails"
        );
        assert!(
            !verify_content(b"a-different-key", CONTENT, &tag),
            "a different key fails"
        );
        assert!(
            !verify_content(KEY, CONTENT, "not-hex!!"),
            "malformed tag fails"
        );
    }

    #[test]
    fn the_tag_is_deterministic() {
        assert_eq!(sign_content(KEY, CONTENT), sign_content(KEY, CONTENT));
        // HMAC-SHA256 is 32 bytes -> 64 hex chars.
        assert_eq!(sign_content(KEY, CONTENT).len(), 64);
    }
}
