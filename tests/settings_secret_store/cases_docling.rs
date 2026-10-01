//! The Docling VLM provider API key singleton, round-tripped on a real database.

use super::support::{
    assert_fails_closed, assert_sealed_under, assert_stored_bytes_exclude, keyed, reset, run,
    service, unkeyed,
};
use context69::{
    contracts::{
        UpdateDoclingConnectionSettings, UpdateDoclingSettingsRequest, UpdateDoclingVlmSettings,
    },
    services::secret_store::key_names,
};
use sqlx::Row;

const SYNTHETIC_KEY: &str = "synthetic-docling-vlm-key";
const PURPOSE: &str = "docling.vlm_api_key";

/// An unreachable Docling endpoint and an unreachable OpenAI-compatible VLM
/// endpoint, so a save can never contact a real service.
fn request(openai_base_url: Option<&str>, api_key: Option<&str>) -> UpdateDoclingSettingsRequest {
    UpdateDoclingSettingsRequest {
        connection: UpdateDoclingConnectionSettings {
            base_url: "http://127.0.0.1:1".to_string(),
            timeout_secs: 5,
            poll_interval_secs: 5,
            task_timeout_secs: 30,
            max_inflight: 1,
        },
        vlm: UpdateDoclingVlmSettings {
            openai_base_url: openai_base_url.map(str::to_string),
            api_key: api_key.map(str::to_string),
            vlm_pipeline_model: Some("context69-secret-store-test".to_string()),
            picture_description_model: Some("context69-secret-store-test".to_string()),
            code_formula_model: Some("context69-secret-store-test".to_string()),
            picture_description_preset: None,
        },
    }
}

async fn legacy_column(db: &context69::db::Database) -> Option<String> {
    let row = sqlx::query("SELECT api_key FROM context69.docling_settings WHERE singleton")
        .fetch_optional(db.pool())
        .await
        .expect("read the legacy column");
    row.map(|row| row.get::<Option<String>, _>("api_key").unwrap_or_default())
}

#[test]
fn the_docling_vlm_api_key_round_trips_sealed_and_mirrors_the_legacy_column() {
    run(async |db| {
        reset(db).await;

        let settings = service(db, keyed(db));
        let saved = settings
            .update_docling_settings(&request(Some("http://127.0.0.1:1/v1"), Some(SYNTHETIC_KEY)))
            .await
            .expect("save the docling vlm key");
        assert!(saved.vlm.has_api_key, "presence must be reported");

        assert_sealed_under(db.pool(), key_names::DOCLING_VLM_API_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::DOCLING_VLM_API_KEY, SYNTHETIC_KEY).await;
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        // The provider config is where the key actually has to arrive, and it is
        // resolved store-first: the key reaches the provider in memory and appears in
        // no response, log, or task payload.
        sqlx::query("UPDATE context69.docling_settings SET api_key = NULL WHERE singleton")
            .execute(db.pool())
            .await
            .expect("blank the mirror");
        let config = settings
            .resolve_docling_config()
            .await
            .expect("resolve the provider config")
            .expect("the docling row is stored");
        assert_eq!(config.vlm.api_key.as_deref(), Some(SYNTHETIC_KEY));
        assert!(
            settings
                .get_docling_settings()
                .await
                .expect("read the redacted settings")
                .vlm
                .has_api_key
        );

        reset(db).await;
    });
}

#[test]
fn a_sealed_docling_vlm_key_fails_closed_and_still_reports_presence() {
    run(async |db| {
        reset(db).await;
        service(db, keyed(db))
            .update_docling_settings(&request(Some("http://127.0.0.1:1/v1"), Some(SYNTHETIC_KEY)))
            .await
            .expect("seed the sealed row");

        let unkeyed_service = service(db, unkeyed(db));
        assert_fails_closed(
            unkeyed_service
                .resolve_docling_config()
                .await
                .map(|config| config.is_some()),
        );
        // The stage step resolves before it validates, so a save on the same
        // deployment stops there too rather than writing an empty credential.
        assert_fails_closed(
            unkeyed_service
                .update_docling_settings(&request(Some("http://127.0.0.1:1/v1"), None))
                .await
                .map(|response| response.vlm.has_api_key),
        );
        assert!(
            unkeyed_service
                .get_docling_settings()
                .await
                .expect("read presence without a master key")
                .vlm
                .has_api_key
        );

        reset(db).await;
    });
}

#[test]
fn keeping_the_vlm_base_url_keeps_the_stored_docling_key() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_docling_settings(&request(Some("http://127.0.0.1:1/v1"), Some(SYNTHETIC_KEY)))
            .await
            .expect("seed the sealed row");

        // A request that keeps the base URL and omits the key is a Keep for both: the
        // stored credential has to survive a save that never mentions it.
        settings
            .update_docling_settings(&request(Some("http://127.0.0.1:1/v1"), None))
            .await
            .expect("save with a Keep");
        assert_sealed_under(db.pool(), key_names::DOCLING_VLM_API_KEY, PURPOSE).await;
        assert_eq!(legacy_column(db).await.as_deref(), Some(SYNTHETIC_KEY));

        reset(db).await;
    });
}
