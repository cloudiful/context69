//! Which credential the embedding connection probe uses, and what it leaves
//! behind: the submitted key, never the stored one, and never a persisted row.

use super::embedding_probe_endpoint::{live_probe_request, recording_embedding_endpoint};
use super::support::{
    assert_sealed_under, assert_stored_bytes_exclude, keyed, reset, run, runtime_request, service,
    store_metadata,
};
use context69::services::secret_store::key_names;

const SYNTHETIC_KEY: &str = "synthetic-embedding-key";
const PURPOSE: &str = "embedding.api_key";

/// A non-persisting probe pointed at an unreachable loopback endpoint, so it can
/// never reach a real provider.
fn probe_request(
    api_key: Option<&str>,
) -> context69::contracts::settings::TestRuntimeEmbeddingRequest {
    context69::contracts::settings::TestRuntimeEmbeddingRequest {
        base_url: "http://127.0.0.1:1/v1".to_string(),
        model: "context69-secret-store-test".to_string(),
        dimensions: 128,
        timeout_secs: 1,
        api_key: api_key.map(str::to_string),
    }
}

#[test]
fn an_embedding_probe_uses_the_submitted_key_without_persisting_it() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        // The probe is pointed at an unreachable loopback port, so it fails at
        // the transport layer; the point is that it never wrote the key.
        let error = settings
            .test_embedding_connection(&probe_request(Some(SYNTHETIC_KEY)))
            .await
            .expect_err("an unreachable probe must fail");
        assert!(
            !error.to_string().contains(SYNTHETIC_KEY),
            "a probe error must never echo the key"
        );
        assert!(
            store_metadata(db.pool(), key_names::EMBEDDING_API_KEY)
                .await
                .is_none(),
            "a probe must not persist the submitted key"
        );
        assert!(
            !settings
                .get_runtime_settings()
                .await
                .expect("read presence")
                .embedding
                .has_api_key,
            "a probe must not make the deployment report a stored key"
        );

        reset(db).await;
    });
}

#[test]
fn an_embedding_probe_without_a_submitted_key_uses_the_stored_one() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the sealed row");

        // No key submitted: the probe resolves the stored one, so the row is
        // opened (and left untouched) rather than reported as unconfigured.
        let result = settings
            .test_embedding_connection(&probe_request(None))
            .await;
        assert!(
            result.is_err(),
            "the unreachable probe still fails at the transport layer"
        );
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;

        reset(db).await;
    });
}

/// The success path of the connection test, and the credential it actually put on
/// the wire: the submitted key, never the stored one and never persisted.
#[test]
fn a_probe_that_succeeds_sent_the_submitted_key_and_stored_nothing() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some("stored-key-not-for-the-probe"), None))
            .await
            .expect("seed a stored key that the probe must ignore");

        let (base_url, recorded) = recording_embedding_endpoint(axum::http::StatusCode::OK).await;
        settings
            .test_embedding_connection(&live_probe_request(&base_url, Some(SYNTHETIC_KEY)))
            .await
            .expect("the recording endpoint answers the probe");

        assert_eq!(
            recorded.bearer_tokens(),
            vec![SYNTHETIC_KEY.to_string()],
            "the probe must authenticate with the submitted key, not the stored one"
        );
        // The stored row is untouched by the probe: still sealed, and still not
        // carrying the submitted key in any form.
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::EMBEDDING_API_KEY, SYNTHETIC_KEY).await;

        reset(db).await;
    });
}

/// The same round trip with no key submitted: the sealed row is opened so the
/// stored credential is what reaches the provider, and the row is left alone.
#[test]
fn a_probe_without_a_key_sent_the_stored_key() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the sealed row");

        let (base_url, recorded) = recording_embedding_endpoint(axum::http::StatusCode::OK).await;
        settings
            .test_embedding_connection(&live_probe_request(&base_url, None))
            .await
            .expect("the recording endpoint answers the probe");

        assert_eq!(
            recorded.bearer_tokens(),
            vec![SYNTHETIC_KEY.to_string()],
            "an absent key must fall back to the sealed stored one"
        );
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;

        reset(db).await;
    });
}

/// A refused probe reports the refusal without echoing the credential it used,
/// and still writes nothing.
#[test]
fn a_refused_probe_reports_the_failure_without_the_submitted_key() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let (base_url, recorded) =
            recording_embedding_endpoint(axum::http::StatusCode::UNAUTHORIZED).await;
        let error = settings
            .test_embedding_connection(&live_probe_request(&base_url, Some(SYNTHETIC_KEY)))
            .await
            .expect_err("a refused probe must fail");
        let message = format!("{error:#}");
        assert!(
            !message.contains(SYNTHETIC_KEY),
            "a probe failure must not echo the submitted key: {message}"
        );
        assert!(
            !message.contains("Bearer"),
            "a probe failure must not echo the request authorization: {message}"
        );
        assert_eq!(recorded.bearer_tokens(), vec![SYNTHETIC_KEY.to_string()]);
        assert!(
            store_metadata(db.pool(), key_names::EMBEDDING_API_KEY)
                .await
                .is_none(),
            "a refused probe must not persist the submitted key"
        );

        reset(db).await;
    });
}
