//! The search / rerank provider API key singleton, round-tripped on a real
//! database.

use super::support::{
    assert_fails_closed, assert_sealed_under, assert_stored_bytes_exclude, keyed, reset, run,
    service, unkeyed,
};
use context69::{
    contracts::{CanonicalUpdateSearchSettingsRequest, SearchMode, SecretPatch},
    services::secret_store::key_names,
};
use sqlx::Row;

const SYNTHETIC_KEY: &str = "synthetic-search-key";
const PURPOSE: &str = "search.api_key";

fn request(api_key: SecretPatch) -> CanonicalUpdateSearchSettingsRequest {
    CanonicalUpdateSearchSettingsRequest {
        mode: SearchMode::Hybrid,
        rerank_enabled: true,
        rerank_base_url: "http://127.0.0.1:1/v1".to_string(),
        rerank_model: "context69-secret-store-test".to_string(),
        candidate_limit: 20,
        timeout_secs: 5,
        api_key,
        vector_weight: 0.55,
        keyword_weight: 0.35,
    }
}

async fn legacy_column(db: &context69::db::Database) -> Option<String> {
    let row = sqlx::query("SELECT api_key FROM context69.search_settings WHERE singleton")
        .fetch_optional(db.pool())
        .await
        .expect("read the legacy column");
    row.map(|row| row.get::<Option<String>, _>("api_key").unwrap_or_default())
}

#[test]
fn the_search_api_key_round_trips_sealed_and_mirrors_the_legacy_column() {
    run(async |db| {
        reset(db).await;

        let settings = service(db, keyed(db));
        let saved = settings
            .update_search_settings(&request(SecretPatch::Set(SYNTHETIC_KEY.to_string())))
            .await
            .expect("save the search key");
        assert!(saved.has_api_key, "presence must be reported");

        assert_sealed_under(db.pool(), key_names::SEARCH_API_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::SEARCH_API_KEY, SYNTHETIC_KEY).await;
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        // Store-first: the redacted projection stays truthful from the sealed row
        // alone, and the response never carries the value back.
        sqlx::query("UPDATE context69.search_settings SET api_key = NULL WHERE singleton")
            .execute(db.pool())
            .await
            .expect("blank the mirror");
        let read = settings.get_search_settings().await.expect("read");
        assert!(read.has_api_key);
        assert!(
            !format!("{read:?}").contains(SYNTHETIC_KEY),
            "a settings response must not carry the key back"
        );

        reset(db).await;
    });
}

#[test]
fn a_sealed_search_key_fails_closed_and_still_reports_presence() {
    run(async |db| {
        reset(db).await;
        service(db, keyed(db))
            .update_search_settings(&request(SecretPatch::Set(SYNTHETIC_KEY.to_string())))
            .await
            .expect("seed the sealed row");

        // Even a Keep has to resolve the current value first, so an unreadable sealed
        // row stops the save rather than being ignored.
        let unkeyed_service = service(db, unkeyed(db));
        assert_fails_closed(
            unkeyed_service
                .update_search_settings(&request(SecretPatch::Keep))
                .await
                .map(|response| response.has_api_key),
        );
        assert!(
            unkeyed_service
                .get_search_settings()
                .await
                .expect("read presence without a master key")
                .has_api_key
        );

        // A clear needs no master key: it removes whatever is stored, so a credential
        // this deployment cannot open is not what blocks removing it.
        unkeyed_service
            .update_search_settings(&request(SecretPatch::Clear))
            .await
            .expect("clear without a master key");
        assert!(
            !service(db, keyed(db))
                .get_search_settings()
                .await
                .expect("read after the clear")
                .has_api_key
        );

        reset(db).await;
    });
}

#[test]
fn keeping_the_search_api_key_leaves_both_stores_untouched() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_search_settings(&request(SecretPatch::Set(SYNTHETIC_KEY.to_string())))
            .await
            .expect("seed the sealed row");

        settings
            .update_search_settings(&request(SecretPatch::Keep))
            .await
            .expect("save with a Keep");
        assert_sealed_under(db.pool(), key_names::SEARCH_API_KEY, PURPOSE).await;
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        // A blank value normalizes to a Keep too: whitespace parity with the legacy
        // wire, and it must not blank the row.
        settings
            .update_search_settings(&request(SecretPatch::Set("   ".to_string())))
            .await
            .expect("save with a blank value");
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        reset(db).await;
    });
}
