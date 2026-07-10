use axum::http::StatusCode;
use serde_json::json;

use super::support::{client, spawn_json};

#[tokio::test]
async fn runtime_settings_get_uses_nested_resource() {
    let response = json!({
        "qdrant": {"url":"http://qdrant","collection_name":"docs","recreate_on_dimension_mismatch":false},
        "embedding": {"base_url":"https://example.test","model":"embed","dimensions":3,"timeout_secs":10,"has_api_key":true},
        "scheduler": {"interval_secs":60,"run_on_start":false,"max_concurrency":1,"job_id":"sync","valkey_url":null},
        "chunking": {"max_chars":1000,"overlap_chars":100},
        "file_library": {"storage_root":"/tmp","max_upload_size_mb":10,"max_upload_request_size_mb":20,"ingest_concurrency":1,"pdf_pages_per_task":5}
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    let settings = client(&base_url)
        .settings()
        .runtime()
        .get()
        .await
        .expect("runtime settings");
    let request = captured.await.expect("captured request");
    assert_eq!(settings.embedding.dimensions, 3);
    assert_eq!(request.uri.path(), "/v1/settings/runtime");
}

#[tokio::test]
async fn docling_settings_get_uses_nested_resource() {
    let response = json!({
        "configured": false, "source": "unconfigured",
        "connection": {"base_url":null,"timeout_secs":10,"poll_interval_secs":1},
        "vlm": {"openai_base_url":null,"has_api_key":false,
            "vlm_pipeline_model":null,"picture_description_model":null,"code_formula_model":null}
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    client(&base_url)
        .settings()
        .docling()
        .get()
        .await
        .expect("docling settings");
    assert_eq!(captured.await.unwrap().uri.path(), "/v1/settings/docling");
}

#[tokio::test]
async fn search_settings_get_uses_nested_resource() {
    let response = json!({
        "mode":"hybrid", "rerank_enabled":false, "rerank_base_url":"", "rerank_model":"",
        "candidate_limit":20, "timeout_secs":10, "has_api_key":false
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    client(&base_url)
        .settings()
        .search()
        .get()
        .await
        .expect("search settings");
    assert_eq!(captured.await.unwrap().uri.path(), "/v1/settings/search");
}
