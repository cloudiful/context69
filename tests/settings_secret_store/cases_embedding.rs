//! The embedding provider API key singleton, round-tripped on a real database.

use super::support::{
    assert_fails_closed, assert_retired_columns_absent, assert_sealed_under,
    assert_stored_bytes_exclude, identity_fingerprint, keyed, reset, run, runtime_request,
    seed_vector_identity, service, store_metadata, unkeyed,
};
use context69::services::secret_store::key_names;

const SYNTHETIC_KEY: &str = "synthetic-embedding-key";
const PURPOSE: &str = "embedding.api_key";

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn the_embedding_api_key_round_trips_sealed_and_leaves_no_plaintext_column() {
    run(async |db| {
        reset(db).await;
        assert_retired_columns_absent(db).await;

        let settings = service(db, keyed(db));
        let saved = settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("save the embedding key");
        assert!(saved.embedding.has_api_key, "presence must be reported");

        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::EMBEDDING_API_KEY, SYNTHETIC_KEY).await;

        // The store is the whole value: the settings row cannot hold a second
        // copy, so the redacted projection stays truthful on its own.
        assert_retired_columns_absent(db).await;
        assert!(
            settings
                .get_runtime_settings()
                .await
                .expect("read")
                .embedding
                .has_api_key
        );
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;

        reset(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_sealed_embedding_key_fails_closed_and_still_reports_presence() {
    run(async |db| {
        reset(db).await;
        service(db, keyed(db))
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the sealed row");

        // A deployment that cannot open the row must be told so, not served an
        // absent credential as if the operator had never configured one.
        let unkeyed_service = service(db, unkeyed(db));
        assert_fails_closed(
            unkeyed_service
                .update_runtime_settings(&runtime_request(None, None))
                .await
                .map(|response| response.embedding.has_api_key),
        );

        // Presence is metadata-only, so it stays truthful on that same deployment.
        assert!(
            unkeyed_service
                .get_runtime_settings()
                .await
                .expect("read presence without a master key")
                .embedding
                .has_api_key
        );
        assert!(
            store_metadata(db.pool(), key_names::EMBEDDING_API_KEY)
                .await
                .is_some()
        );

        reset(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_absent_embedding_key_keeps_the_stored_one() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the sealed row");

        // A settings request carries no clear for this field, so an absent key is a
        // Keep: the stored credential must survive a save that omits it.
        settings
            .update_runtime_settings(&runtime_request(None, None))
            .await
            .expect("save without a key");
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        assert!(
            settings
                .get_runtime_settings()
                .await
                .expect("read after the keep")
                .embedding
                .has_api_key,
            "a save that omits the key must not drop the stored one"
        );

        reset(db).await;
    });
}

/// The optional query model is part of the stored embedding configuration and
/// round-trips unchanged.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn the_query_model_round_trips() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let mut request = runtime_request(Some(SYNTHETIC_KEY), None);
        request.embedding.query_model = Some("context69-query-alias".to_string());
        let saved = settings
            .update_runtime_settings(&request)
            .await
            .expect("save the query model");
        assert_eq!(
            saved.embedding.query_model.as_deref(),
            Some("context69-query-alias")
        );

        let read = settings
            .get_runtime_settings()
            .await
            .expect("read the saved configuration");
        assert_eq!(
            read.embedding.query_model.as_deref(),
            Some("context69-query-alias")
        );

        // A blank query model is the document model, so it is stored as none.
        let mut blank = runtime_request(None, None);
        blank.embedding.query_model = Some("   ".to_string());
        let cleared = settings
            .update_runtime_settings(&blank)
            .await
            .expect("save a blank query model");
        assert_eq!(cleared.embedding.query_model, None);

        reset(db).await;
    });
}

/// An invalid upload limit is refused before anything is written, so the stored
/// configuration and the stored credential stay exactly as they were.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_invalid_upload_limit_leaves_the_settings_and_the_secret_unchanged() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the stored configuration");

        let before = settings
            .get_runtime_settings()
            .await
            .expect("read the stored configuration");

        let mut invalid = runtime_request(Some("rotated-key"), None);
        invalid.file_library.max_upload_size_mb = 0;
        let error = settings
            .update_runtime_settings(&invalid)
            .await
            .expect_err("a zero upload limit must be refused");
        assert!(error.to_string().contains("max_upload_size_mb"), "{error}");

        let after = settings
            .get_runtime_settings()
            .await
            .expect("read the stored configuration again");
        assert_eq!(
            after.file_library.max_upload_size_mb,
            before.file_library.max_upload_size_mb
        );
        assert_eq!(after.embedding.model, before.embedding.model);
        assert_eq!(after.embedding.query_model, before.embedding.query_model);

        // The credential was not rotated either: the refused save wrote nothing.
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::EMBEDDING_API_KEY, "rotated-key").await;

        reset(db).await;
    });
}

/// A query model is not part of the vector space, so changing it never demands
/// a document rebuild.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_query_model_change_does_not_demand_a_document_rebuild() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the stored configuration");
        // A deployment without an active collection owes an explicit rebuild,
        // so the case starts from an installation that already has one: only
        // then can a query-only change be shown to owe nothing.
        let baseline_fingerprint =
            identity_fingerprint("http://127.0.0.1:1/v1", "context69-secret-store-test", 128);
        seed_vector_identity(db, &baseline_fingerprint, 128).await;
        let baseline = settings
            .get_runtime_settings()
            .await
            .expect("read the stored configuration");
        assert!(
            !baseline.requires_rebuild,
            "an installation whose active collection matches owes no rebuild"
        );

        let mut query_only = runtime_request(None, None);
        query_only.embedding.query_model = Some("context69-query-alias".to_string());
        let saved = settings
            .update_runtime_settings(&query_only)
            .await
            .expect("a query model change is not an identity change");
        assert_eq!(
            saved.embedding.query_model.as_deref(),
            Some("context69-query-alias")
        );
        assert!(
            !saved.requires_rebuild,
            "a query model change does not change the vector space"
        );

        reset(db).await;
    });
}
