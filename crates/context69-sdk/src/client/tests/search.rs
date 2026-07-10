use axum::http::{Method, StatusCode};
use context69_contracts::SearchRequest;
use serde_json::json;

use super::support::{client, spawn_json};

#[tokio::test]
async fn search_executes_borrowed_request() {
    let response = json!({"query": "rust", "hits": []});
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    let request_body = SearchRequest {
        query: "rust".to_string(),
        limit: 5,
        source_key: None,
        group_path: None,
        published_after: None,
        published_before: None,
    };
    let result = client(&base_url)
        .search()
        .execute(&request_body)
        .await
        .expect("search");
    let request = captured.await.expect("captured request");
    assert_eq!(result.query, "rust");
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.uri.path(), "/v1/search");
}

#[tokio::test]
async fn user_directory_encodes_query_parameters() {
    let (base_url, captured) = spawn_json(StatusCode::OK, &json!([])).await;
    client(&base_url)
        .user_directory()
        .search("alice ops", 12)
        .await
        .expect("search directory");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.uri.path(), "/v1/user-directory");
    assert_eq!(request.uri.query(), Some("query=alice+ops&limit=12"));
}

#[tokio::test]
async fn document_resource_gets_document() {
    let response = json!({
        "document_id": 42, "group_key":"ops", "group_path":"ops", "visibility":"private",
        "source_key":"alerts", "external_id":"42", "title":"Incident", "summary":null,
        "source_uri":"https://example.test/42", "published_at":null,
        "updated_at":"2026-07-10T00:00:00Z", "record_hash":"abc", "metadata_json":{},
        "library_file_id":null, "library_section_label":null, "library_path":null,
        "is_library_file":false, "chunks":[]
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    let document = client(&base_url)
        .document(42)
        .get()
        .await
        .expect("get document");
    assert_eq!(document.document_id, 42);
    assert_eq!(captured.await.unwrap().uri.path(), "/v1/documents/42");
}
