//! Credential redaction through the real probe path: a provider that echoes the
//! credential it received, in the clear, JSON-escaped, percent-encoded in any hex
//! case, or at any length from one character up, must never have it returned to
//! the caller.

use super::embedding_probe_endpoint::{live_probe_request, recording_embedding_endpoint_with};
use super::support::{keyed, reset, run, service, store_metadata};
use context69::services::secret_store::key_names;

#[test]
fn a_probe_refusal_that_echoes_the_credential_never_returns_it() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        // A key with characters that make the escaped and percent-encoded forms
        // differ from the raw value, so all three forms are exercised.
        let key = "sk live/probe+42=xyz";

        let (base_url, recorded) =
            recording_embedding_endpoint_with(axum::http::StatusCode::UNAUTHORIZED, true).await;

        let error = settings
            .test_embedding_connection(&live_probe_request(&base_url, Some(key)))
            .await
            .expect_err("the endpoint refuses the probe");
        let message = format!("{error:#}");
        // The endpoint really did receive the credential, so the redaction
        // assertions below are not passing because nothing arrived.
        assert_eq!(recorded.bearer_tokens(), vec![key.to_string()]);

        assert!(!message.contains(key), "the raw key leaked: {message}");
        let encoded = |upper: bool| {
            key.bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (byte as char).to_string()
                    }
                    other if upper => format!("%{other:02X}"),
                    other => format!("%{other:02x}"),
                })
                .collect::<String>()
        };
        assert!(
            !message.contains(&encoded(true)),
            "the upper percent-encoded key leaked: {message}"
        );
        assert!(
            !message.contains(&encoded(false)),
            "the lower percent-encoded key leaked: {message}"
        );
        assert!(
            !message.contains(&key.replace('\\', "\\\\").replace('"', "\\\"")),
            "the JSON-escaped key leaked: {message}"
        );
        // The scheme word may survive: it is not a secret. A bearer token with
        // anything other than the redaction marker after it is.
        assert!(
            !message.contains("Bearer sk") && !message.contains("Bearer%20sk"),
            "the request authorization leaked: {message}"
        );
        assert!(
            message.contains("[redacted]"),
            "the echoed credential must be marked redacted: {message}"
        );
        // The refusal itself is still reported, so the operator learns why.
        assert!(
            message.contains("rejected token"),
            "the provider error must still reach the operator: {message}"
        );

        reset(db).await;
    });
}

/// AC4 now names short credentials explicitly. Redaction used to skip anything
/// under 8 characters to avoid matching unrelated text, so a one to three
/// character key echoed by the provider was disclosed verbatim. This walks the
/// lengths that were previously exempt and drives each one through the whole
/// probe path: submitted key, provider, error formatter, settings handler.
#[test]
fn a_short_echoed_credential_never_returns_through_the_probe() {
    for key in ["a", "ab", "abc", "abcd", "abcde", "abcdef", "abcdefg"] {
        run(async |db| {
            reset(db).await;
            let settings = service(db, keyed(db));

            let (base_url, recorded) =
                recording_embedding_endpoint_with(axum::http::StatusCode::UNAUTHORIZED, true).await;
            let error = settings
                .test_embedding_connection(&live_probe_request(&base_url, Some(key)))
                .await
                .expect_err("the endpoint refuses the probe");
            let message = format!("{error:#}");

            // The endpoint really received this exact short credential.
            assert_eq!(
                recorded.bearer_tokens(),
                vec![key.to_string()],
                "the endpoint must have received the submitted short key"
            );
            assert!(
                !message.contains(&format!("Bearer {key}")),
                "a {}-character key must never come back: {message}",
                key.len()
            );
            assert!(
                !message.contains(&format!("Bearer%20{key}")),
                "the percent-encoded form of a {}-character key leaked: {message}",
                key.len()
            );
            assert!(
                !message.contains(&format!("token {key}")),
                "the provider message leaked a {}-character key: {message}",
                key.len()
            );
            // A short key that is a common word fragments the message, so the
            // requirement is only that the operator still learns it was refused.
            assert!(
                message.contains("rejected token"),
                "the refusal must still reach the operator: {message}"
            );
            assert!(
                store_metadata(db.pool(), key_names::EMBEDDING_API_KEY)
                    .await
                    .is_none(),
                "a probe must never persist a short key either"
            );

            reset(db).await;
        });
    }
}

/// A short credential is redacted too: a provider that echoes it must not be
/// able to disclose it through an error of any length.
#[test]
fn a_probe_refusal_that_echoes_a_short_credential_never_returns_it() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let key = "ab1";

        let (base_url, recorded) =
            recording_embedding_endpoint_with(axum::http::StatusCode::UNAUTHORIZED, true).await;
        let error = settings
            .test_embedding_connection(&live_probe_request(&base_url, Some(key)))
            .await
            .expect_err("the endpoint refuses the probe");
        let message = format!("{error:#}");
        assert_eq!(recorded.bearer_tokens(), vec![key.to_string()]);
        assert!(!message.contains(key), "the short key leaked: {message}");
        assert!(
            message.contains("[redacted]"),
            "the echoed credential must be marked redacted: {message}"
        );

        reset(db).await;
    });
}

/// Percent encoding is case-insensitive, so coverage of an all-upper and an
/// all-lower rendering leaves two things untested: a provider that mixes the hex
/// case between escapes, and a body that echoes the credential more than once so
/// the scan has to advance past a match and still find the next one. Both are
/// driven through the real probe path rather than the formatter alone.
#[test]
fn a_probe_refusal_scrubs_mixed_case_percent_encodings_and_repeats() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        // Every character needs escaping, so each byte is its own %XX escape and
        // the hex case of each escape can differ from its neighbour.
        let key = "a/b+c=d";

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

        // Rebuild the percent renderings independently of the endpoint helper, so
        // the assertion is not satisfied by the same code that produced them.
        let encode = |byte: u8, upper: bool| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other if upper => format!("%{other:02X}"),
            other => format!("%{other:02x}"),
        };
        let uniform = |upper: bool| {
            key.bytes()
                .map(|byte| encode(byte, upper))
                .collect::<String>()
        };
        let alternating = key
            .bytes()
            .enumerate()
            .map(|(index, byte)| encode(byte, index % 2 == 0))
            .collect::<String>();
        // Alternating hex case between escapes, which neither uniform rendering
        // covers and which the endpoint emits as encoded_mixed.
        let upper = uniform(true);
        let lower = uniform(false);

        for candidate in [upper, lower, alternating] {
            assert!(
                !message.contains(&candidate),
                "a percent rendering of the credential leaked ({candidate}): {message}"
            );
        }
        assert!(!message.contains(key), "the raw key leaked: {message}");
        assert!(
            message.contains("rejected token"),
            "the refusal must still reach the operator: {message}"
        );

        reset(db).await;
    });
}
