//! Bounded error output through the real probe path: a credential pushed across
//! the error-preview cut leaves no prefix, and a provider error of hundreds of
//! kilobytes is neither returned nor logged whole.

use super::embedding_probe_endpoint::{
    live_probe_request, recording_embedding_endpoint_large_error,
    recording_embedding_endpoint_padded, recording_embedding_endpoint_with,
};
use super::support::{keyed, reset, run, service};

/// KNOWN LIMITATION, pinned so it cannot regress silently. The error formatter
/// truncates the upstream body to 320 characters and redacts only afterwards, so
/// a credential that straddles the cut has its leading portion disclosed. The
/// whole credential is never returned, which is what the security property needs,
/// but a prefix can be, and this test cannot make that deterministic through a
/// loopback endpoint. What it does pin is that the two projections an operator
/// reads are safe: the whole credential never appears, and the untruncated
/// provider message is always fully redacted.
#[test]
fn a_credential_straddling_the_preview_truncation_is_never_fully_disclosed() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let key = "sk live/probe+42=xyz";

        let (base_url, recorded) =
            recording_embedding_endpoint_with(axum::http::StatusCode::UNAUTHORIZED, true).await;
        let error = settings
            .test_embedding_connection(&live_probe_request(&base_url, Some(key)))
            .await
            .expect_err("the endpoint refuses the probe");
        let message = format!("{error:#}");
        assert_eq!(
            recorded.bearer_tokens(),
            vec![key.to_string()],
            "the endpoint must have received the submitted key"
        );

        // The whole credential never comes back, in any projection.
        assert!(!message.contains(key), "the raw key leaked: {message}");
        // The untruncated provider_error projection is fully redacted.
        assert!(
            message.contains("provider_error=rejected token Bearer [redacted]"),
            "the untruncated provider message must be fully redacted: {message}"
        );

        reset(db).await;
    });
}

/// The preview boundary is now closed: redaction runs before the 320-character
/// cut, so a credential pushed across that cut must leave neither the whole
/// value nor any prefix of it. Driven through the real probe path with padding
/// sized to place the echoed credential across the boundary.
#[test]
fn a_credential_across_the_preview_boundary_leaves_no_prefix() {
    // The error preview keeps 320 characters (src/embedding PREVIEW_CHARS).
    const PREVIEW_CHARS: usize = 320;
    for key in [
        "sk live/probe+42=xyz",
        "sk-boundary-secret-abcdefghijklmnop",
        "a/b+c=d",
    ] {
        run(async |db| {
            reset(db).await;
            let settings = service(db, keyed(db));

            // Calibrate once with no filler so the offset of the echoed credential in the
            // serialized body is measured rather than guessed, then re-probe with
            // filler sized to place that occurrence across the preview cut.
            let (calibrate_url, calibrator) =
                recording_embedding_endpoint_padded(axum::http::StatusCode::UNAUTHORIZED, 0).await;
            settings
                .test_embedding_connection(&live_probe_request(&calibrate_url, Some(key)))
                .await
                .expect_err("the calibration probe is refused too");
            let echoed = format!("Bearer {key}");
            let bare = calibrator
                .last_body()
                .expect("the calibration body was recorded");
            let bare_offset = bare
                .find(&echoed)
                .expect("the endpoint echoed the credential");
            // Aim for the credential to start five characters before the cut, so
            // it always straddles regardless of its length.
            let padding = (PREVIEW_CHARS - 5).saturating_sub(bare_offset);

            let (base_url, recorded) =
                recording_embedding_endpoint_padded(axum::http::StatusCode::UNAUTHORIZED, padding)
                    .await;
            let error = settings
                .test_embedding_connection(&live_probe_request(&base_url, Some(key)))
                .await
                .expect_err("the endpoint refuses the probe");
            let message = format!("{error:#}");
            assert_eq!(
                recorded.bearer_tokens(),
                vec![key.to_string()],
                "the endpoint must have received the submitted key"
            );

            // Prove the fixture really straddles the preview window using the body
            // the endpoint actually sent, so the case cannot pass merely because
            // the credential was nowhere near the cut.
            let body = recorded
                .last_body()
                .expect("the endpoint recorded the body it sent");
            let start = body
                .find(&echoed)
                .expect("the endpoint echoed the credential");
            assert!(
                start < PREVIEW_CHARS && start + echoed.chars().count() > PREVIEW_CHARS,
                "the credential must straddle the preview window: start={start} window={PREVIEW_CHARS}"
            );
            // Truncating first would have disclosed exactly this prefix.
            let naive_prefix: String = body[start..].chars().take(PREVIEW_CHARS - start).collect();
            assert!(
                naive_prefix.len() >= 4
                    && echoed.starts_with(&naive_prefix)
                    && naive_prefix.len() < echoed.len(),
                "the naive truncation must have leaked a proper prefix: {naive_prefix:?}"
            );

            assert!(
                !message.contains(key),
                "the whole credential leaked: {message}"
            );
            for prefix_len in 4..=key.len().min(12) {
                assert!(
                    !message.contains(&key[..prefix_len]),
                    "a {prefix_len}-character credential prefix leaked: {message}"
                );
            }
            // Percent-encoded renderings across the same boundary, upper, lower
            // and mixed.
            let encode_byte = |byte: u8, upper: bool| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (byte as char).to_string()
                }
                other if upper => format!("%{other:02X}"),
                other => format!("%{other:02x}"),
            };
            for rendering in [
                key.bytes()
                    .map(|b| encode_byte(b, true))
                    .collect::<String>(),
                key.bytes()
                    .map(|b| encode_byte(b, false))
                    .collect::<String>(),
                key.bytes()
                    .enumerate()
                    .map(|(i, b)| encode_byte(b, i % 2 == 0))
                    .collect::<String>(),
            ] {
                assert!(
                    !message.contains(&rendering),
                    "a percent rendering leaked across the boundary: {message}"
                );
            }
            // The error text stays bounded: a long or heavily redacted body
            // cannot inflate what is returned or logged.
            assert!(
                message.len() < 4096,
                "the returned error must stay bounded: {} bytes",
                message.len()
            );
            assert!(
                message.contains("rejected token"),
                "the refusal must still reach the operator: {message}"
            );

            reset(db).await;
        });
    }
}

/// Upper bound on an embedding error string the settings handler can return.
/// Both the response preview and the provider-error projection are capped at 320
/// characters and the retry wrapper repeats the inner text a bounded number of
/// times, so a few kilobytes is generous while still being a real cap.
const MAX_EMBEDDING_ERROR_BYTES: usize = 16 * 1024;

/// Drives the probe against a provider that answers with a refusal whose
/// `error.message` is hundreds of kilobytes long. The credential is echoed raw
/// and in every percent-encoding hex case, so the case fails if the provider-error
/// projection is returned whole or if a credential survives it. Exercised through
/// the real probe endpoint, which is the path the settings handler and the batch
/// logger both surface.
async fn assert_large_provider_error_is_bounded_and_redacted(
    db: &context69::db::Database,
    status: axum::http::StatusCode,
    label: &str,
) {
    const PROVIDER_ERROR_BLOB: usize = 300 * 1024;

    let settings = service(db, keyed(db));
    let key = "sk live/probe+42=xyz";
    let (base_url, recorded) =
        recording_embedding_endpoint_large_error(status, PROVIDER_ERROR_BLOB).await;
    let error = settings
        .test_embedding_connection(&live_probe_request(&base_url, Some(key)))
        .await
        .expect_err("the large provider error must fail the probe");
    let message = format!("{error:#}");
    // A retryable status is attempted more than once, so require every recorded
    // attempt to carry the same credential rather than exactly one attempt.
    let tokens = recorded.bearer_tokens();
    assert!(!tokens.is_empty(), "the endpoint recorded no attempt");
    assert!(
        tokens.iter().all(|token| token == key),
        "every attempt must carry the submitted key: {tokens:?}"
    );

    // The body the endpoint really sent was large, so the case cannot pass
    // because the provider stayed quiet.
    let body = recorded
        .last_body()
        .expect("the endpoint recorded the body it sent");
    assert!(
        body.len() > PROVIDER_ERROR_BLOB / 2,
        "the fixture must be a large provider error: {} bytes",
        body.len()
    );

    assert!(
        message.len() < MAX_EMBEDDING_ERROR_BYTES,
        "{label}: the returned error must stay bounded, got {} bytes",
        message.len()
    );
    assert!(
        !message.contains(key),
        "{label}: the raw key leaked: {message}"
    );
    let encode_byte = |byte: u8, upper: bool| match byte {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
            (byte as char).to_string()
        }
        other if upper => format!("%{other:02X}"),
        other => format!("%{other:02x}"),
    };
    for rendering in [
        key.bytes()
            .map(|b| encode_byte(b, true))
            .collect::<String>(),
        key.bytes()
            .map(|b| encode_byte(b, false))
            .collect::<String>(),
        key.bytes()
            .enumerate()
            .map(|(i, b)| encode_byte(b, i % 2 == 0))
            .collect::<String>(),
    ] {
        assert!(
            !message.contains(&rendering),
            "{label}: a percent rendering leaked: {message}"
        );
    }
    // The operator still learns the refusal happened, and the marker shows the
    // credential was recognised rather than merely dropped.
    assert!(
        message.contains("rejected token"),
        "{label}: the refusal must still reach the operator: {message}"
    );
    assert!(
        message.contains("[redacted]"),
        "{label}: the echoed credential must be marked redacted: {message}"
    );
}

#[test]
fn a_large_http_provider_error_is_bounded_and_redacted() {
    // The HTTP-error path: a retryable server error, so
    // format_embedding_http_error builds the message the handler returns.
    run(async |db| {
        assert_large_provider_error_is_bounded_and_redacted(
            db,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "http error path",
        )
        .await;
        reset(db).await;
    });
}

#[test]
fn a_large_parse_error_provider_message_is_bounded_and_redacted() {
    // The parse path: a success status whose body is not a usable embedding
    // payload, so parse_embedding_response builds the message instead.
    run(async |db| {
        assert_large_provider_error_is_bounded_and_redacted(
            db,
            axum::http::StatusCode::OK,
            "parse error path",
        )
        .await;
        reset(db).await;
    });
}
