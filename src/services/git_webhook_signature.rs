//! GitHub webhook signature verification for the provider ingress (issue #681
//! work unit 4B1).
//!
//! GitHub signs a delivery with HMAC-SHA256 over the exact raw request body and
//! sends the digest in `X-Hub-Signature-256` as `sha256=<64 lowercase hex>`. The
//! check has to run on the bytes exactly as they arrived — any re-serialization
//! of a parsed body changes them — so this module takes `&[u8]` and never sees a
//! parsed value.
//!
//! Two properties matter and neither is optional:
//!
//! * **Constant-time comparison.** The supplied digest is compared through
//!   [`Mac::verify_slice`], which folds the difference over the whole tag with a
//!   constant-time primitive rather than short-circuiting on the first differing
//!   byte. A byte-by-byte `==` would leak how much of a guessed signature is
//!   correct.
//! * **Bounded inputs.** The header value, the delivery id, and the body all
//!   have a fixed maximum before any cryptographic work, so an unauthenticated
//!   caller cannot force unbounded allocation or a long parse.
//!
//! Nothing here returns a reason: a missing, malformed, or mismatched signature
//! is one boolean, so a caller cannot turn it into a distinguishing message. The
//! signing secret is never logged or formatted, and the computed digest is
//! compared in place rather than stored.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Longest accepted `X-Hub-Signature-256` value: the `sha256=` prefix plus 64
/// hexadecimal characters. GitHub sends nothing longer, so a longer value is
/// refused before it is parsed.
pub const MAX_GIT_WEBHOOK_SIGNATURE_CHARS: usize = "sha256=".len() + 64;

/// Longest accepted provider delivery id. GitHub sends a UUID; the column that
/// stores it is unbounded, so the bound lives at this boundary instead.
pub const MAX_GIT_WEBHOOK_DELIVERY_ID_CHARS: usize = 128;

/// Longest accepted `{external_hook_id}` path segment.
pub const MAX_GIT_WEBHOOK_HOOK_ID_CHARS: usize = 255;

/// Largest raw webhook body this ingress reads. The slice records idempotency
/// only and never parses the event, so the bound is deliberately smaller than a
/// provider's own payload ceiling.
pub const MAX_GIT_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

type HmacSha256 = Hmac<Sha256>;

/// Verifies a GitHub `sha256=` signature over the exact raw body bytes with
/// constant-time comparison.
///
/// Returns `false` for a missing, oversized, non-`sha256=`, non-hex, or
/// mismatched value; the caller cannot tell which from the result.
pub fn verify_github_signature(signing_secret: &[u8], body: &[u8], signature_header: &str) -> bool {
    let Some(expected) = parse_github_signature(signature_header) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(signing_secret) else {
        return false;
    };
    mac.update(body);
    // `verify_slice` compares in constant time and rejects a wrong length.
    mac.verify_slice(&expected).is_ok()
}

/// Parses the `sha256=<64 hex>` form into the raw digest.
///
/// `None` for any other shape, so a caller cannot accidentally compare a
/// prefix, a truncated digest, or a different algorithm.
pub fn parse_github_signature(header: &str) -> Option<[u8; 32]> {
    if header.len() > MAX_GIT_WEBHOOK_SIGNATURE_CHARS {
        return None;
    }
    decode_hex_32(header.strip_prefix("sha256=")?)
}

/// Whether a header value could name a delivery: non-empty and within the bound.
pub fn is_valid_delivery_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_GIT_WEBHOOK_DELIVERY_ID_CHARS
}

/// Whether a path segment could name a hook: non-empty and within the bound.
pub fn is_valid_hook_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_GIT_WEBHOOK_HOOK_ID_CHARS
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        let high = hex_nibble(hex.as_bytes()[index * 2])?;
        let low = hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Some(digest)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_GIT_WEBHOOK_SIGNATURE_CHARS, is_valid_delivery_id, is_valid_hook_id,
        parse_github_signature, verify_github_signature,
    };
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    /// RFC 4231 test case 2: key `Jefe`, data `what do ya want for nothing?`.
    const RFC4231_KEY: &[u8] = b"Jefe";
    const RFC4231_BODY: &[u8] = b"what do ya want for nothing?";
    const RFC4231_DIGEST: &str = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(body);
        format!(
            "sha256={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    }

    #[test]
    fn a_published_rfc4231_vector_verifies() {
        // The expected digest is external to this implementation, so this pins
        // the byte order and the algorithm, not just self-consistency.
        let header = format!("sha256={RFC4231_DIGEST}");
        assert!(verify_github_signature(RFC4231_KEY, RFC4231_BODY, &header));
    }

    #[test]
    fn a_wrong_secret_or_mutated_body_is_rejected() {
        let header = sign(RFC4231_KEY, RFC4231_BODY);
        assert!(
            !verify_github_signature(b"wrong-key", RFC4231_BODY, &header),
            "a different signing secret must not verify"
        );
        assert!(
            !verify_github_signature(RFC4231_KEY, b"what do ya want for nothing!", &header),
            "a body mutated by one byte must not verify"
        );
    }

    #[test]
    fn a_missing_or_malformed_signature_is_rejected() {
        let valid = sign(RFC4231_KEY, RFC4231_BODY);
        let malformed = [
            "",
            "sha256=",
            "sha256=00",
            "sha1=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "SHA256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec384g",
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec384",
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec38433",
        ];
        for header in malformed {
            assert!(
                !verify_github_signature(RFC4231_KEY, RFC4231_BODY, header),
                "malformed signature must be rejected: {header:?}"
            );
        }
        assert!(verify_github_signature(RFC4231_KEY, RFC4231_BODY, &valid));
    }

    #[test]
    fn a_header_over_the_bound_is_refused_before_parsing() {
        let oversized = format!("sha256={}", "a".repeat(MAX_GIT_WEBHOOK_SIGNATURE_CHARS));
        assert!(oversized.len() > MAX_GIT_WEBHOOK_SIGNATURE_CHARS);
        assert!(parse_github_signature(&oversized).is_none());
        assert!(!verify_github_signature(
            RFC4231_KEY,
            RFC4231_BODY,
            &oversized
        ));
    }

    #[test]
    fn a_same_length_tag_is_rejected_wherever_it_differs() {
        // Constant-time comparison must not depend on the position of the first
        // wrong byte: both an early and a late difference are rejected.
        let valid = RFC4231_DIGEST;
        for index in [0, 31, 63] {
            let mut bytes = valid.as_bytes().to_vec();
            let original = bytes[index];
            bytes[index] = if original == b'0' { b'1' } else { b'0' };
            let header = format!("sha256={}", String::from_utf8(bytes).expect("ascii"));
            assert!(
                !verify_github_signature(RFC4231_KEY, RFC4231_BODY, &header),
                "a same-length tag must be rejected at byte {index}"
            );
        }
        assert!(verify_github_signature(
            RFC4231_KEY,
            RFC4231_BODY,
            &format!("sha256={valid}")
        ));
    }

    #[test]
    fn delivery_and_hook_ids_are_bounded() {
        assert!(is_valid_delivery_id("d1e2a3b4-0000-0000-0000-000000000000"));
        assert!(!is_valid_delivery_id(""));
        assert!(!is_valid_delivery_id(
            &"d".repeat(super::MAX_GIT_WEBHOOK_DELIVERY_ID_CHARS + 1)
        ));
        assert!(is_valid_hook_id("12345"));
        assert!(!is_valid_hook_id(""));
        assert!(!is_valid_hook_id(
            &"h".repeat(super::MAX_GIT_WEBHOOK_HOOK_ID_CHARS + 1)
        ));
    }
}
