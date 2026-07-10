use axum::http::{Method, StatusCode};
use context69_contracts::{SourceConfigInput, UpsertSourceConnectionRequest};
use serde_json::json;

use super::support::{client, spawn_empty, spawn_json};

fn source_json() -> serde_json::Value {
    json!({
        "group_key": "personal",
        "group_path": "personal",
        "visibility": "private",
        "source_key": "alerts/prod",
        "display_name": "Alerts",
        "description": null,
        "example_queries": [],
        "connection": "warehouse",
        "has_database_url": true,
        "origin_status": "connected",
        "origin_message": null,
        "sync_strategy": "incremental",
        "connector_type": "postgres_sql",
        "base_query": "select 1",
        "batch_size": 100,
        "last_cursor_updated_at": null,
        "last_cursor_external_id": null,
        "last_success_at": null
    })
}

fn source_request() -> SourceConfigInput {
    SourceConfigInput {
        source_key: "alerts/prod".to_string(),
        display_name: Some("Alerts".to_string()),
        description: None,
        example_queries: vec![],
        connection: "warehouse".to_string(),
        database_url: None,
        sync_strategy: "incremental".to_string(),
        connector_type: "postgres_sql".to_string(),
        base_query: "select 1".to_string(),
        batch_size: 100,
        visibility: None,
    }
}

#[tokio::test]
async fn sources_collection_creates() {
    let (base_url, captured) = spawn_json(StatusCode::CREATED, &source_json()).await;
    let source = client(&base_url)
        .sources()
        .create(&source_request())
        .await
        .expect("create source");
    let request = captured.await.expect("captured request");
    assert_eq!(source.source_key, "alerts/prod");
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.uri.path(), "/v1/sources");
}

#[tokio::test]
async fn source_resource_encodes_key_on_update() {
    let (base_url, captured) = spawn_json(StatusCode::OK, &source_json()).await;
    client(&base_url)
        .source("alerts/prod")
        .update(&source_request())
        .await
        .expect("update source");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::PUT);
    assert_eq!(request.uri.path(), "/v1/sources/alerts%2Fprod");
}

#[tokio::test]
async fn source_connections_update_uses_collection_endpoint() {
    let input = UpsertSourceConnectionRequest {
        name: "main/warehouse".to_string(),
        database_url: Some("postgres://example".to_string()),
    };
    let response = json!({
        "name": "main/warehouse",
        "has_database_url": true,
        "origin_status": "unknown",
        "origin_message": null
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    client(&base_url)
        .source_connections()
        .update(&input)
        .await
        .expect("update connection");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::PUT);
    assert_eq!(request.uri.path(), "/v1/source-connections");
}

#[tokio::test]
async fn source_connection_delete_encodes_name() {
    let (base_url, captured) = spawn_empty(StatusCode::NO_CONTENT).await;
    client(&base_url)
        .source_connection("main/warehouse")
        .delete()
        .await
        .expect("delete connection");
    let request = captured.await.expect("captured request");
    assert_eq!(
        request.uri.path(),
        "/v1/source-connections/main%2Fwarehouse"
    );
}
