//! The independent Git code embedding credential, round-tripped on a real
//! database.
//!
//! The code block owns its own purpose and key name, so every case here checks
//! that it never borrows the document credential. Disabled rows keep their
//! non-secret fields and sealed key without opening it, while an enabled row
//! keeps failing closed.

use super::embedding_probe_endpoint::{
    recording_embedding_endpoint, recording_malformed_success_endpoint,
};
use super::support::{
    assert_sealed_under, assert_stored_bytes_exclude, keyed, reset_guarded, run, runtime_request,
    service, store_metadata, unkeyed,
};
use context69::contracts::SecretPatch;
use context69::contracts::settings::{
    TestRuntimeEmbeddingPurpose, TestRuntimeEmbeddingRequest, UpdateCodeEmbeddingSettings,
};
use context69::services::secret_store::key_names;

const CODE_KEY: &str = "synthetic-code-embedding-key";
const DOCUMENT_KEY: &str = "synthetic-document-embedding-key";
const CODE_PURPOSE: &str = "code_embedding.api_key";

fn code_update(enabled: bool, api_key: SecretPatch) -> UpdateCodeEmbeddingSettings {
    UpdateCodeEmbeddingSettings {
        enabled,
        base_url: "http://127.0.0.1:1/v1".to_string(),
        model: "code-model".to_string(),
        query_model: None,
        dimensions: 2,
        timeout_secs: 5,
        api_key,
    }
}

fn code_request(
    enabled: bool,
    api_key: SecretPatch,
) -> context69::contracts::UpdateRuntimeSettingsRequest {
    let mut request = runtime_request(None, None);
    request.code_embedding = Some(code_update(enabled, api_key));
    request
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn the_code_key_round_trips_sealed_under_the_code_purpose_only() {
    run(async |db| {
        reset_guarded(db).await;
        let settings = service(db, keyed(db));

        let saved = settings
            .update_runtime_settings(&code_request(true, SecretPatch::Set(CODE_KEY.to_string())))
            .await
            .expect("save the code key");
        let code = saved
            .code_embedding
            .as_ref()
            .expect("the code block is stored");
        assert!(code.enabled);
        assert!(code.has_api_key, "presence must be reported");

        assert_sealed_under(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_KEY).await;
        // The document credential is a different purpose and stays absent.
        assert!(
            store_metadata(db.pool(), key_names::EMBEDDING_API_KEY)
                .await
                .is_none(),
            "a code save must not create a document credential"
        );
        assert!(!saved.embedding.has_api_key);

        let row = db
            .get_runtime_code_embedding_settings()
            .await
            .expect("read the code row")
            .expect("the code row exists");
        assert!(row.enabled);
        assert_eq!(row.model, "code-model");
        assert!(row.api_key.is_none(), "the row never carries the key");

        let snapshot = settings
            .code_embedding_snapshot()
            .expect("an enabled code row installs a provider");
        assert_eq!(snapshot.config.api_key.as_deref(), Some(CODE_KEY));

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn keep_and_outer_omission_preserve_the_code_secret() {
    run(async |db| {
        reset_guarded(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&code_request(true, SecretPatch::Set(CODE_KEY.to_string())))
            .await
            .expect("seed the code key");

        settings
            .update_runtime_settings(&code_request(true, SecretPatch::Keep))
            .await
            .expect("a keep save preserves the code key");
        assert_sealed_under(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_PURPOSE).await;
        assert_eq!(
            settings
                .code_embedding_snapshot()
                .expect("the code provider is still installed")
                .config
                .api_key
                .as_deref(),
            Some(CODE_KEY)
        );

        // An ordinary document save omits the code block entirely: it must not
        // touch the code row, its credential, or the live code snapshot.
        settings
            .update_runtime_settings(&runtime_request(None, None))
            .await
            .expect("a save without the code block preserves it");
        assert_sealed_under(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_PURPOSE).await;
        assert!(
            settings
                .get_runtime_settings()
                .await
                .expect("read presence")
                .code_embedding
                .as_ref()
                .is_some_and(|code| code.enabled && code.has_api_key)
        );
        assert_eq!(
            settings
                .code_embedding_snapshot()
                .expect("an omitted block leaves the snapshot alone")
                .config
                .api_key
                .as_deref(),
            Some(CODE_KEY)
        );

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_disabled_row_keeps_an_unreadable_code_key_and_can_be_cleared_without_a_master_key() {
    run(async |db| {
        reset_guarded(db).await;
        service(db, keyed(db))
            .update_runtime_settings(&code_request(true, SecretPatch::Set(CODE_KEY.to_string())))
            .await
            .expect("seed an enabled code row with a sealed key");

        // A deployment that cannot open the retained key can still disable the
        // block with a Keep: it needs no provider and must not decrypt it.
        let unkeyed_settings = service(db, unkeyed(db));
        let disabled = unkeyed_settings
            .update_runtime_settings(&code_request(false, SecretPatch::Keep))
            .await
            .expect("disabling code embedding must not open the retained code key");
        let code = disabled
            .code_embedding
            .as_ref()
            .expect("the disabled non-secret fields stay stored");
        assert!(!code.enabled);
        assert!(
            code.has_api_key,
            "presence metadata survives the disable without decryption"
        );
        assert!(
            unkeyed_settings.code_embedding_snapshot().is_none(),
            "a disabled row publishes no provider"
        );
        assert_sealed_under(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_PURPOSE).await;

        // An explicit Clear removes the unreadable key without opening it either.
        let cleared = unkeyed_settings
            .update_runtime_settings(&code_request(false, SecretPatch::Clear))
            .await
            .expect("clearing the code key must not require the master key");
        assert!(
            !cleared
                .code_embedding
                .as_ref()
                .expect("the disabled row remains")
                .has_api_key,
            "the cleared key is no longer present"
        );
        assert!(
            store_metadata(db.pool(), key_names::CODE_EMBEDDING_API_KEY)
                .await
                .is_none(),
            "an explicit Clear removes the sealed row without decrypting it"
        );
        assert!(
            unkeyed_settings.code_embedding_snapshot().is_none(),
            "a cleared code row still publishes no provider"
        );

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_malformed_success_code_probe_redacts_endpoint_and_provider_credentials() {
    run(async |db| {
        reset_guarded(db).await;
        let key = "synthetic-code-probe-key";
        let user = "synthetic-probe-user";
        let password = "synthetic-probe-password";
        let query = "synthetic-probe-query";
        let (address, recorded) = recording_malformed_success_endpoint(key.to_string()).await;
        let authority = address
            .strip_prefix("http://")
            .expect("the endpoint reports an HTTP URL");
        let base_url = format!("http://{user}:{password}@{authority}/v1?token={query}");

        let settings = service(db, keyed(db));
        let error = settings
            .test_embedding_connection(&TestRuntimeEmbeddingRequest {
                purpose: TestRuntimeEmbeddingPurpose::Code,
                base_url,
                model: "code-model".to_string(),
                query_model: None,
                dimensions: 2,
                timeout_secs: 5,
                api_key: Some(key.to_string()),
            })
            .await
            .expect_err("a malformed success body must fail the code probe");
        let message = format!("{error:#}");

        let headers = recorded.recorded_authorization_headers();
        assert_eq!(headers.len(), 1, "the endpoint really received the probe");
        assert!(
            headers[0]
                .as_deref()
                .is_some_and(|header| header.starts_with("Basic ")),
            "the endpoint received the endpoint userinfo: {:?}",
            headers[0]
        );
        for marker in [key, user, password, query] {
            assert!(
                !message.contains(marker),
                "the code probe error leaked {marker}: {message}"
            );
        }
        assert!(
            message.contains("failed to parse embedding response"),
            "{message}"
        );
        assert!(
            message.contains("endpoint=http://127.0.0.1:"),
            "the sanitized endpoint must stay useful: {message}"
        );
        assert!(message.contains("model=code-model"), "{message}");
        assert!(message.contains("body_preview"), "{message}");
        assert!(
            message.contains("[redacted]"),
            "the echoed provider credential must be redacted: {message}"
        );

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_invalid_code_probe_url_does_not_expose_endpoint_credentials() {
    run(async |db| {
        reset_guarded(db).await;
        let marker = "synthetic-invalid-url-marker";
        let settings = service(db, keyed(db));
        let error = settings
            .test_embedding_connection(&TestRuntimeEmbeddingRequest {
                purpose: TestRuntimeEmbeddingPurpose::Code,
                base_url: format!("http://user:{marker}@127.0.0.1:bad/v1"),
                model: "code-model".to_string(),
                query_model: None,
                dimensions: 2,
                timeout_secs: 1,
                api_key: Some("synthetic-other-key".to_string()),
            })
            .await
            .expect_err("the invalid endpoint must be rejected");
        let leaked = format!("{error:#}").contains(marker);
        reset_guarded(db).await;
        assert!(
            !leaked,
            "the invalid endpoint error exposes URL credentials"
        );
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_failed_code_probe_does_not_expose_endpoint_query() {
    run(async |db| {
        reset_guarded(db).await;
        let marker = "synthetic-endpoint-query-marker";
        let settings = service(db, keyed(db));
        let error = settings
            .test_embedding_connection(&TestRuntimeEmbeddingRequest {
                purpose: TestRuntimeEmbeddingPurpose::Code,
                base_url: format!("http://127.0.0.1:1/v1?token={marker}"),
                model: "code-model".to_string(),
                query_model: None,
                dimensions: 2,
                timeout_secs: 1,
                api_key: None,
            })
            .await
            .expect_err("the endpoint refuses connections");
        let leaked = format!("{error:#}").contains(marker);
        reset_guarded(db).await;
        assert!(!leaked, "the connection error exposes its endpoint query");
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_failed_code_probe_hides_endpoint_userinfo_password_query_and_key() {
    run(async |db| {
        reset_guarded(db).await;
        let user = "synthetic-transport-user";
        let password = "synthetic-transport-password";
        let query = "synthetic-transport-query";
        let key = "synthetic-transport-api-key";
        let settings = service(db, keyed(db));
        let error = settings
            .test_embedding_connection(&TestRuntimeEmbeddingRequest {
                purpose: TestRuntimeEmbeddingPurpose::Code,
                base_url: format!("http://{user}:{password}@127.0.0.1:1/v1?token={query}"),
                model: "code-model".to_string(),
                query_model: None,
                dimensions: 2,
                timeout_secs: 1,
                api_key: Some(key.to_string()),
            })
            .await
            .expect_err("the endpoint refuses connections");
        let message = format!("{error:#}");
        reset_guarded(db).await;

        for marker in [user, password, query, key] {
            assert!(
                !message.contains(marker),
                "the code probe transport error leaked {marker}: {message}"
            );
        }
        assert!(
            message.contains("embedding upstream transport error"),
            "{message}"
        );
        assert!(
            message.contains("endpoint=http://127.0.0.1:1/v1"),
            "the sanitized endpoint must stay useful: {message}"
        );
        assert!(message.contains("model=code-model"), "{message}");
        assert!(message.contains("source_chain"), "{message}");
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_code_probe_uses_only_the_code_purpose() {
    run(async |db| {
        reset_guarded(db).await;
        let (base_url, recorded) = recording_embedding_endpoint(axum::http::StatusCode::OK).await;
        let settings = service(db, keyed(db));

        let mut document = runtime_request(Some(DOCUMENT_KEY), None);
        document.embedding.base_url = base_url.clone();
        settings
            .update_runtime_settings(&document)
            .await
            .expect("seed the document credential");
        let mut code = code_request(true, SecretPatch::Set(CODE_KEY.to_string()));
        code.code_embedding
            .as_mut()
            .expect("the code block")
            .base_url = base_url.clone();
        settings
            .update_runtime_settings(&code)
            .await
            .expect("seed the code credential");

        // No submitted key: the code purpose resolves its own stored credential,
        // never the document one, even though both are readable.
        settings
            .test_embedding_connection(&TestRuntimeEmbeddingRequest {
                purpose: TestRuntimeEmbeddingPurpose::Code,
                base_url: base_url.clone(),
                model: "code-model".to_string(),
                query_model: None,
                dimensions: 2,
                timeout_secs: 5,
                api_key: None,
            })
            .await
            .expect("the recording endpoint answers the code probe");
        assert_eq!(
            recorded.bearer_tokens(),
            vec![CODE_KEY.to_string()],
            "the code probe must never use the document credential"
        );

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_code_probe_without_a_code_credential_does_not_fall_back_to_the_document_key() {
    run(async |db| {
        reset_guarded(db).await;
        let (base_url, recorded) = recording_embedding_endpoint(axum::http::StatusCode::OK).await;
        let settings = service(db, keyed(db));

        // Only the document credential exists.
        let mut document = runtime_request(Some(DOCUMENT_KEY), None);
        document.embedding.base_url = base_url.clone();
        settings
            .update_runtime_settings(&document)
            .await
            .expect("seed the document credential");

        settings
            .test_embedding_connection(&TestRuntimeEmbeddingRequest {
                purpose: TestRuntimeEmbeddingPurpose::Code,
                base_url,
                model: "code-model".to_string(),
                query_model: None,
                dimensions: 2,
                timeout_secs: 5,
                api_key: None,
            })
            .await
            .expect("the recording endpoint answers without authentication");
        let headers = recorded.recorded_authorization_headers();
        assert_eq!(headers.len(), 1);
        assert!(
            headers[0].is_none(),
            "a code probe must not borrow the document credential: {:?}",
            headers[0]
        );

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_code_secret_write_fault_publishes_no_provider_and_recovers() {
    run(async |db| {
        reset_guarded(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(DOCUMENT_KEY), None))
            .await
            .expect("seed the document settings");
        assert!(settings.code_embedding_snapshot().is_none());

        // A fixture-owned trigger rejects only the code secret insert, so the
        // secret step fails after provider construction but before the row and
        // the snapshot are published.
        for sql in [
            "CREATE OR REPLACE FUNCTION context69.fixture_reject_code_secret() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN IF NEW.key = 'code_embedding_api_key' THEN RAISE \
             EXCEPTION 'injected code secret failure'; END IF; RETURN NEW; END $$",
            "CREATE TRIGGER fixture_reject_code_secret BEFORE INSERT ON \
             context69.internal_secrets FOR EACH ROW EXECUTE FUNCTION \
             context69.fixture_reject_code_secret()",
        ] {
            sqlx::query(sql)
                .execute(db.pool())
                .await
                .expect("install the fixture secret fault trigger");
        }

        let failed = settings
            .update_runtime_settings(&code_request(true, SecretPatch::Set(CODE_KEY.to_string())))
            .await;
        let error = failed.expect_err("the injected secret failure must abort the save");

        for sql in [
            "DROP TRIGGER IF EXISTS fixture_reject_code_secret ON context69.internal_secrets",
            "DROP FUNCTION IF EXISTS context69.fixture_reject_code_secret()",
        ] {
            sqlx::query(sql)
                .execute(db.pool())
                .await
                .expect("remove the fixture secret fault trigger");
        }

        assert!(
            error.to_string().contains("injected code secret failure"),
            "{error}"
        );
        assert!(
            settings.code_embedding_snapshot().is_none(),
            "a failed secret write must not publish a provider"
        );
        assert!(
            db.get_runtime_code_embedding_settings()
                .await
                .expect("read the code row")
                .is_none(),
            "a failed secret write must not publish the code row"
        );
        assert!(
            store_metadata(db.pool(), key_names::CODE_EMBEDDING_API_KEY)
                .await
                .is_none(),
            "the rejected secret was never stored"
        );

        // The same save succeeds once the fault is gone.
        settings
            .update_runtime_settings(&code_request(true, SecretPatch::Set(CODE_KEY.to_string())))
            .await
            .expect("the recovered save installs the code block");
        assert!(settings.code_embedding_snapshot().is_some());
        assert_sealed_under(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_PURPOSE).await;

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_code_only_save_changes_no_document_identity_or_rebuild_state() {
    run(async |db| {
        reset_guarded(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(DOCUMENT_KEY), None))
            .await
            .expect("seed the document settings");
        let before = settings
            .get_runtime_settings()
            .await
            .expect("read the document settings");

        let mut code = runtime_request(None, None);
        code.code_embedding = Some(code_update(true, SecretPatch::Set(CODE_KEY.to_string())));
        let after = settings
            .update_runtime_settings(&code)
            .await
            .expect("save the code block");

        assert_eq!(after.embedding.base_url, before.embedding.base_url);
        assert_eq!(after.embedding.model, before.embedding.model);
        assert_eq!(after.embedding.query_model, before.embedding.query_model);
        assert_eq!(after.embedding.dimensions, before.embedding.dimensions);
        assert_eq!(after.embedding.has_api_key, before.embedding.has_api_key);
        assert_eq!(after.requires_rebuild, before.requires_rebuild);
        assert_eq!(after.qdrant.url, before.qdrant.url);
        assert_eq!(after.qdrant.collection_name, before.qdrant.collection_name);
        assert!(
            after
                .code_embedding
                .as_ref()
                .is_some_and(|code| code.enabled)
        );
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, "embedding.api_key").await;

        reset_guarded(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_combined_write_fault_rolls_back_both_rows_and_publishes_no_snapshot() {
    run(async |db| {
        reset_guarded(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(DOCUMENT_KEY), None))
            .await
            .expect("seed the document settings");
        let before = settings
            .get_runtime_settings()
            .await
            .expect("read the baseline settings");
        assert!(settings.code_embedding_snapshot().is_none());

        // A fixture-owned trigger makes only the code row write fail, so the
        // document rows and the code row must roll back together.
        for sql in [
            "CREATE OR REPLACE FUNCTION context69.fixture_reject_code_row() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected code row failure'; END $$",
            "CREATE TRIGGER fixture_reject_code_row BEFORE INSERT OR UPDATE ON \
             context69.runtime_code_embedding_settings FOR EACH ROW EXECUTE FUNCTION \
             context69.fixture_reject_code_row()",
        ] {
            sqlx::query(sql)
                .execute(db.pool())
                .await
                .expect("install the fixture fault trigger");
        }

        let mut changed = code_request(true, SecretPatch::Set(CODE_KEY.to_string()));
        changed.scheduler.interval_secs = before.scheduler.interval_secs + 1;
        let failed = settings.update_runtime_settings(&changed).await;
        let error = failed.expect_err("the injected code-row failure must abort the save");

        for sql in [
            "DROP TRIGGER IF EXISTS fixture_reject_code_row ON \
             context69.runtime_code_embedding_settings",
            "DROP FUNCTION IF EXISTS context69.fixture_reject_code_row()",
        ] {
            sqlx::query(sql)
                .execute(db.pool())
                .await
                .expect("remove the fixture fault trigger");
        }

        assert!(
            error.to_string().contains("injected code row failure"),
            "{error}"
        );
        let after = settings
            .get_runtime_settings()
            .await
            .expect("read the settings after the failure");
        assert_eq!(after.embedding.base_url, before.embedding.base_url);
        assert_eq!(after.embedding.model, before.embedding.model);
        assert_eq!(after.qdrant.url, before.qdrant.url);
        assert_eq!(
            after.scheduler.interval_secs,
            before.scheduler.interval_secs
        );
        assert!(
            after.code_embedding.is_none(),
            "the code row was inserted in the same transaction and must roll back"
        );
        assert!(
            settings.code_embedding_snapshot().is_none(),
            "a failed save must not publish a code provider"
        );

        // The same save succeeds once the fault is gone, so recovery is real.
        let recovered = settings
            .update_runtime_settings(&changed)
            .await
            .expect("the recovered save installs the code block");
        assert_eq!(
            recovered.scheduler.interval_secs,
            changed.scheduler.interval_secs
        );
        assert!(settings.code_embedding_snapshot().is_some());
        assert_sealed_under(db.pool(), key_names::CODE_EMBEDDING_API_KEY, CODE_PURPOSE).await;

        reset_guarded(db).await;
    });
}
