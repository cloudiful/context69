use axum::http::{Method, StatusCode};
use context69_contracts::UpsertProviderAccountRequest;
use serde_json::json;

use super::support::{client, spawn_empty, spawn_json};

#[tokio::test]
async fn runtime_settings_get_uses_nested_resource() {
    let response = json!({
        "qdrant": {"url":"http://qdrant","collection_name":"docs","recreate_on_dimension_mismatch":false},
        "embedding": {"provider_account_key":"openai","model":"embed","dimensions":3,"timeout_secs":10},
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
async fn provider_accounts_update_uses_collection_endpoint() {
    let input = UpsertProviderAccountRequest {
        account_key: "openai/main".to_string(),
        provider_kind: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        base_url: "https://example.test".to_string(),
        api_key: None,
        clear_api_key: false,
        disabled: false,
    };
    let response = json!({
        "account_key":"openai/main", "provider_kind":"openai", "display_name":"OpenAI",
        "base_url":"https://example.test", "has_api_key":false, "disabled_at":null
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    client(&base_url)
        .settings()
        .provider_accounts()
        .update(&input)
        .await
        .expect("update provider");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::PUT);
    assert_eq!(request.uri.path(), "/v1/settings/provider-accounts");
}

#[tokio::test]
async fn provider_account_delete_encodes_key() {
    let (base_url, captured) = spawn_empty(StatusCode::NO_CONTENT).await;
    client(&base_url)
        .settings()
        .provider_account("openai/main")
        .delete()
        .await
        .expect("delete provider");
    assert_eq!(
        captured.await.unwrap().uri.path(),
        "/v1/settings/provider-accounts/openai%2Fmain"
    );
}

#[tokio::test]
async fn docling_settings_get_uses_nested_resource() {
    let response = json!({
        "configured": false, "source": "unconfigured",
        "connection": {"base_url":null,"timeout_secs":10,"poll_interval_secs":1},
        "vlm": {"provider_account_key":null,"openai_base_url":null,"has_api_key":false,
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
