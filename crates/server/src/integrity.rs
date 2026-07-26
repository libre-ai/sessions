//! SP-B content integrity: bind each ingested chunk's text to a keyed signature
//! (HMAC-SHA256) so a chunk can be proven to be the one ingested, unaltered.
//!
//! The grounding-verifier credits a question only against source text it can
//! trust; an integrity tag makes tampering (in the store, in transit) detectable
//! — any change to the text or the tag fails verification. The key is the
//! server's ingestion secret (env-provided in production), never logged.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Hex-encoded SHA-256 over `fields`, each field length-prefixed with its byte
/// length as a big-endian `u64` before its bytes.
///
/// The length prefix is the whole point: it makes the encoding injective, so
/// distinct field vectors cannot collide by concatenation. Without it
/// `["ab", "c"]` and `["a", "bc"]` hash identically, and an attacker who
/// controls a field boundary can move bytes across it while preserving the
/// digest. This function feeds the anti-tamper control hash of an approved
/// claim and the `space_id`-scoped source/artifact hashes, so that property is
/// load-bearing for both grounding integrity and cross-space isolation.
///
/// It is deliberately keyless: it binds *structure*, not authenticity. Use
/// [`sign_content`] when the goal is to prove that stored bytes are the bytes
/// that were ingested.
pub fn hash_fields(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    to_hex(&hasher.finalize())
}

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

    // Reference digests below were produced outside this crate, so they check
    // the framing against an independent oracle rather than against itself:
    //   printf '\x00...\x02ab\x00...\x01c' | shasum -a 256
    const AB_C: &str = "601d5476e2ccfe2c87a2bba7a322659734a05749d5b5aa781f513e4912db0d5f";
    const A_BC: &str = "3fafa1cf2f19a7c1129beb20cf0983f73a489a221fc0dd2f16d1be292d089205";
    const NAIVE_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn hash_fields_matches_the_length_prefixed_framing() {
        assert_eq!(hash_fields(&["ab", "c"]), AB_C);
        assert_eq!(hash_fields(&["a", "bc"]), A_BC);
    }

    #[test]
    fn hash_fields_is_injective_across_field_boundaries() {
        // The regression this guards: dropping the length prefix makes both of
        // these collapse onto sha256("abc"), silently, with no compile error.
        assert_ne!(
            hash_fields(&["ab", "c"]),
            hash_fields(&["a", "bc"]),
            "moving a byte across a field boundary must change the digest"
        );
        assert_ne!(
            hash_fields(&["ab", "c"]),
            NAIVE_ABC,
            "the digest must not be that of the naive concatenation"
        );
        assert_ne!(hash_fields(&["a", "bc"]), NAIVE_ABC);
    }

    #[test]
    fn hash_fields_is_deterministic_and_hex() {
        let digest = hash_fields(&["space-1", "section-1", "text"]);
        assert_eq!(digest, hash_fields(&["space-1", "section-1", "text"]));
        assert_eq!(digest.len(), 64, "SHA-256 is 32 bytes -> 64 hex chars");
        assert!(digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        assert_ne!(
            digest,
            hash_fields(&["space-2", "section-1", "text"]),
            "the space_id scoping must change the digest"
        );
    }

    #[test]
    fn hash_fields_distinguishes_empty_fields_from_absent_ones() {
        assert_ne!(hash_fields(&["", "a"]), hash_fields(&["a"]));
        assert_ne!(hash_fields(&[]), hash_fields(&[""]));
    }

    #[test]
    fn every_ingested_chunk_gets_a_verifiable_tag() {
        // The "signed integrity hash present per chunk" property: each chunk a
        // document is split into (real corpus chunking) receives its own tag that
        // verifies — none is left unsigned.
        let chunks = presto_rag::corpus::chunk("doc", "Alpha para.\n\nBeta para.\n\nGamma para.");
        assert_eq!(chunks.len(), 3);
        for c in &chunks {
            let tag = sign_content(KEY, &c.text);
            assert!(
                verify_content(KEY, &c.text, &tag),
                "chunk {} unsigned",
                c.source_section_id
            );
            assert_eq!(tag.len(), 64);
        }
    }
}
