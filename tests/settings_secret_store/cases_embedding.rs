//! The embedding provider API key singleton, round-tripped on a real database.

use super::support::{
    assert_fails_closed, assert_sealed_under, assert_stored_bytes_exclude, keyed, reset, run,
    runtime_request, service, store_metadata, unkeyed,
};
use context69::services::secret_store::key_names;
use sqlx::Row;

const SYNTHETIC_KEY: &str = "synthetic-embedding-key";
const PURPOSE: &str = "embedding.api_key";

async fn legacy_column(db: &context69::db::Database) -> Option<String> {
    let row =
        sqlx::query("SELECT api_key FROM context69.runtime_embedding_settings WHERE singleton")
            .fetch_optional(db.pool())
            .await
            .expect("read the legacy column");
    row.map(|row| row.get::<Option<String>, _>("api_key").unwrap_or_default())
}

#[test]
fn the_embedding_api_key_round_trips_sealed_and_mirrors_the_legacy_column() {
    run(async |db| {
        reset(db).await;

        let settings = service(db, keyed(db));
        let saved = settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("save the embedding key");
        assert!(saved.embedding.has_api_key, "presence must be reported");

        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::EMBEDDING_API_KEY, SYNTHETIC_KEY).await;
        // The legacy mirror is what makes the transition rollback-safe: a deployment
        // rolled back to the previous release still resolves the same value.
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        // Store-first: the value the settings service resolves comes from the sealed
        // row, not from the mirror. Clearing the mirror alone must not change it.
        sqlx::query(
            "UPDATE context69.runtime_embedding_settings SET api_key = NULL WHERE singleton",
        )
        .execute(db.pool())
        .await
        .expect("blank the mirror");
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
fn a_sealed_embedding_key_fails_closed_and_still_reports_presence() {
    run(async |db| {
        reset(db).await;
        service(db, keyed(db))
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the sealed row");

        // A deployment that cannot open the row must be told so, and must not be
        // served the legacy value as if it were the current one.
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
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        reset(db).await;
    });
}
