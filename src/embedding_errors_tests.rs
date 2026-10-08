//! Tests for the embedding error formatters and their redaction/bounding.
//!
//! The provider-error projection is the seam a hostile upstream error body
//! reaches through, so these cases assert the returned text: no credential in
//! any rendering survives, redaction runs before every cut, and each projection
//! stays bounded whatever the provider sends.

use super::*;
use crate::domain_errors::find_domain_error;

fn assert_variant(error: &Error, matcher: impl Fn(&DomainError) -> bool, label: &str) {
    let found = find_domain_error(error);
    assert!(
        found.is_some_and(&matcher),
        "expected {label}, got {found:?}: {error}"
    );
}

#[test]
fn embedding_upstream_failures_are_typed_for_api_mapping() {
    let rate_limited = format_embedding_http_error(
        StatusCode::TOO_MANY_REQUESTS,
        "http://embed.local/v1",
        "model",
        "application/json",
        "{}",
        None,
    );
    assert_variant(
        &rate_limited,
        |error| matches!(error, DomainError::RateLimited(_)),
        "rate_limited",
    );

    let upstream = format_embedding_http_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "http://embed.local/v1",
        "model",
        "application/json",
        "{}",
        None,
    );
    assert_variant(
        &upstream,
        |error| matches!(error, DomainError::UpstreamError(_)),
        "upstream_error",
    );

    let client = format_embedding_http_error(
        StatusCode::BAD_REQUEST,
        "http://embed.local/v1",
        "model",
        "application/json",
        "{}",
        None,
    );
    assert_variant(
        &client,
        |error| matches!(error, DomainError::Internal(_)),
        "internal",
    );

    let oversized = oversized_response_error(
        16,
        "http://embed.local/v1",
        "model",
        b"0123456789abcdef",
        None,
    );
    assert_variant(
        &oversized,
        |error| matches!(error, DomainError::PayloadTooLarge(_)),
        "payload_too_large",
    );

    let timeout = format_embedding_attempt_timeout("http://embed.local/v1", "model", 1);
    assert_variant(
        &timeout,
        |error| matches!(error, DomainError::UpstreamTimeout(_)),
        "upstream_timeout",
    );
}

#[test]
fn an_http_error_body_cannot_echo_the_submitted_key() {
    let key = "sk-live-secret-value-123";
    let body = format!(r#"{{"error":{{"message":"invalid key {key}"}},"echo":"Bearer {key}"}}"#);

    let error = format_embedding_http_error(
        StatusCode::UNAUTHORIZED,
        "http://embed.local/v1",
        "model",
        "application/json",
        &body,
        Some(key),
    );
    let message = error.to_string();

    assert!(!message.contains(key), "{message}");
    assert!(message.contains("[redacted]"), "{message}");
    assert!(!message.contains("Bearer sk-live"), "{message}");
}

#[test]
fn a_short_key_is_redacted_too() {
    // AC4 requires redaction for a credential of any length, so even a
    // two-character key echoed by a hostile provider is scrubbed.
    assert_eq!(redact_secret("boom ab", Some("ab")), "boom [redacted]");
    assert_eq!(redact_secret("token=xy", Some("xy")), "token=[redacted]");
    // An absent or empty key leaves the text unchanged.
    assert_eq!(redact_secret("abc", None), "abc");
    assert_eq!(redact_secret("abc", Some("")), "abc");
}

#[test]
fn a_short_echoed_key_is_redacted_in_http_errors() {
    let key = "ab1";
    let body = format!(r#"{{"error":"bad token {key}"}}"#);
    let error = format_embedding_http_error(
        StatusCode::UNAUTHORIZED,
        "http://embed.local/v1",
        "model",
        "application/json",
        &body,
        Some(key),
    );
    let message = error.to_string();
    assert!(!message.contains(key), "{message}");
    assert!(message.contains("[redacted]"), "{message}");
}

#[test]
fn percent_encoded_credentials_are_redacted_in_any_hex_case() {
    let key = "sk/live+x";
    // Uppercase, lowercase, and mixed-case hex renderings of the same
    // percent-encoded key.
    assert_eq!(redact_secret("sk%2Flive%2Bx", Some(key)), "[redacted]");
    assert_eq!(redact_secret("sk%2flive%2bx", Some(key)), "[redacted]");
    assert_eq!(redact_secret("sk%2flive%2Bx", Some(key)), "[redacted]");
    // A mixed rendering inside surrounding text is scrubbed in place.
    assert_eq!(
        redact_secret("GET /v1?key=sk%2Flive%2bx denied", Some(key)),
        "GET /v1?key=[redacted] denied"
    );
    // A key whose escape is already lowercase does not need a second pass.
    assert_eq!(redact_secret("a%2fb", Some("a/b")), "[redacted]");
}

#[test]
fn a_lowercase_percent_encoding_does_not_survive_an_http_error() {
    let key = "sk/live";
    let body = r#"{"error":"rejected token sk%2flive at the provider"}"#;
    let error = format_embedding_http_error(
        StatusCode::UNAUTHORIZED,
        "http://embed.local/v1",
        "model",
        "application/json",
        body,
        Some(key),
    );
    let message = error.to_string();
    assert!(!message.contains("sk%2flive"), "{message}");
    assert!(!message.contains(key), "{message}");
    assert!(message.contains("[redacted]"), "{message}");
}

#[test]
fn redaction_runs_before_the_preview_is_truncated() {
    // A long body whose 320-character preview cut lands inside the echoed
    // credential: redacting the whole body first is what keeps a prefix
    // from surviving the cut.
    let key = "sk-boundary-secret-abcdefghijklmnop";
    let padding = "y".repeat(PREVIEW_CHARS - 5);
    let body = format!("{padding}{key}");

    let preview = redact_then_truncate(&body, PREVIEW_CHARS, Some(key));
    assert!(
        !preview.contains(key),
        "the whole credential survived: {preview}"
    );
    assert!(
        !preview.contains(&key[..8]),
        "a credential prefix survived the cut: {preview}"
    );
    assert!(
        preview.chars().count() <= PREVIEW_CHARS + 3,
        "the preview must stay bounded: {} chars",
        preview.chars().count()
    );

    // Truncating first (the old order) would have left the credential's
    // opening characters, which is exactly what this guards against.
    let naive: String = body.chars().take(PREVIEW_CHARS).collect();
    assert!(
        naive.contains(&key[..5]),
        "the fixture must straddle the boundary: {naive}"
    );
}

#[test]
fn an_http_error_does_not_disclose_a_credential_at_the_preview_boundary() {
    let key = "sk-boundary-secret-abcdefghijklmnop";
    // Position the credential so the 320-character preview cut falls inside
    // it, mirroring a provider error body that echoes a long key.
    let prefix = r#"{"error":"provider rejected "#;
    let key_start = prefix.chars().count() + 5;
    let padding = "y".repeat(PREVIEW_CHARS.saturating_sub(key_start));
    let body = format!("{prefix}{padding}{key}");

    let head = &body[..PREVIEW_CHARS];
    let key_index = body.find(key).expect("the fixture contains the key");
    let leaked = &head[key_index..];
    assert!(
        leaked.len() >= 4,
        "the fixture must straddle the boundary, leaked={leaked:?}"
    );

    let error = format_embedding_http_error(
        StatusCode::UNAUTHORIZED,
        "http://embed.local/v1",
        "model",
        "application/json",
        &body,
        Some(key),
    );
    let message = error.to_string();
    assert!(
        !message.contains(key),
        "the whole credential leaked: {message}"
    );
    assert!(
        !message.contains(leaked),
        "a credential prefix leaked across the boundary: {message}"
    );
    assert!(
        message.len() < PREVIEW_CHARS + 1024,
        "the redacted error must stay bounded: {} bytes",
        message.len()
    );
}

#[test]
fn a_large_provider_error_is_redacted_then_bounded() {
    // Reviewer note 11722: `extract_error_message` can return a ~1 MiB
    // `error.message`/object that was appended without any bound even
    // though `body_preview` is capped. The extracted text must be redacted
    // before truncation and capped to PROVIDER_ERROR_CHARS.
    let key = "sk live/large+secret=xyz";
    let filler = "x".repeat(100_000);
    // Percent renderings of the same key in upper, lower, and mixed hex
    // case, built independently of `redact_secret` so the assertions are
    // not satisfied by the same code that produced them.
    let encode = |byte: u8, upper: bool| match byte {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
            (byte as char).to_string()
        }
        other if upper => format!("%{other:02X}"),
        other => format!("%{other:02x}"),
    };
    let upper: String = key.bytes().map(|b| encode(b, true)).collect();
    let lower: String = key.bytes().map(|b| encode(b, false)).collect();
    let mixed: String = key
        .bytes()
        .enumerate()
        .map(|(i, b)| encode(b, i % 2 == 0))
        .collect();
    assert_ne!(upper, key, "the fixture key must need percent encoding");
    assert_ne!(lower, upper, "upper and lower renderings must differ");
    let large_message = format!("rejected token Bearer {key} {upper} {lower} {mixed} {filler}");
    let body = serde_json::json!({
        "error": { "message": large_message },
    })
    .to_string();
    assert!(
        body.len() > 50_000,
        "the fixture must be large: {} bytes",
        body.len()
    );

    let error = format_embedding_http_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "http://embed.local/v1",
        "model",
        "application/json",
        &body,
        Some(key),
    );
    let message = error.to_string();
    assert!(!message.contains(key), "the raw key leaked");
    for rendering in [&upper, &lower, &mixed] {
        assert!(
            !message.contains(rendering.as_str()),
            "a percent rendering leaked ({rendering}): {message:.200}"
        );
    }
    assert!(message.contains("[redacted]"), "{message:.200}");
    assert!(
        message.contains("rejected token"),
        "useful diagnostics must survive the bound: {message:.200}"
    );
    assert!(
        message.len() < PREVIEW_CHARS + PROVIDER_ERROR_CHARS + 1024,
        "the large provider error must stay bounded: {} bytes",
        message.len()
    );

    // The retry/final wrappers repeat the message: they must stay bounded
    // and redacted too once the source is bounded.
    let finalized = finalize_embedding_error(error, 4, 1_000, 5_000).to_string();
    assert!(!finalized.contains(key), "the key leaked via finalize");
    assert!(
        finalized.len() < PREVIEW_CHARS + PROVIDER_ERROR_CHARS + 2048,
        "the finalized error must stay bounded: {} bytes",
        finalized.len()
    );

    // The shared helper preserves the same ordering for the parse path,
    // which formats the identical `provider_error=` projection.
    let bounded = bound_provider_error(&large_message, Some(key));
    assert!(!bounded.contains(key), "the helper leaked the key");
    assert!(
        bounded.chars().count() <= PROVIDER_ERROR_CHARS + 3,
        "the helper must stay bounded: {} chars",
        bounded.chars().count()
    );
    assert!(
        bounded.contains("rejected token"),
        "the helper must preserve diagnostics: {bounded:.200}"
    );
}
