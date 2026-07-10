use axum::http::StatusCode;
use context69_contracts::{ApiErrorResponse, SearchRequest};

use super::{
    super::{Context69Client, transport::parse_api_error_message},
    support::{client, spawn_json},
};
use crate::Error;

#[test]
fn builder_normalizes_base_url_with_trailing_slash() {
    let client = Context69Client::builder()
        .base_url("http://localhost:8096")
        .expect("base url")
        .build()
        .expect("client");
    assert_eq!(
        client.url("/healthz").expect("healthz url").as_str(),
        "http://localhost:8096/healthz"
    );
}

#[test]
fn builder_rejects_non_pat_token() {
    let error = Context69Client::builder()
        .base_url("http://localhost:8096")
        .expect("base url")
        .with_personal_access_token("plain-access-token")
        .expect_err("invalid token should fail");
    assert!(matches!(error, Error::InvalidPersonalAccessToken(_)));
}

#[test]
fn parses_api_error_body() {
    assert_eq!(
        parse_api_error_message(r#"{"error":"missing bearer token"}"#),
        Some("missing bearer token".to_string())
    );
}

#[tokio::test]
async fn protected_api_requires_pat() {
    let client = Context69Client::builder()
        .base_url("http://localhost:8096")
        .expect("base url")
        .build()
        .expect("client");
    let error = client.groups().list().await.expect_err("requires PAT");
    assert!(matches!(error, Error::AuthenticationRequired));
}

#[tokio::test]
async fn unauthorized_response_does_not_refresh() {
    let (base_url, _) = spawn_json(
        StatusCode::UNAUTHORIZED,
        &ApiErrorResponse {
            error: "expired".to_string(),
        },
    )
    .await;
    let error = client(&base_url)
        .search()
        .execute(&search_request("unauthorized"))
        .await
        .expect_err("should fail");
    match error {
        Error::HttpStatus {
            status, api_error, ..
        } => {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(api_error.as_deref(), Some("expired"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

fn search_request(query: &str) -> SearchRequest {
    SearchRequest {
        query: query.to_string(),
        limit: 8,
        source_key: None,
        group_path: None,
        published_after: None,
        published_before: None,
    }
}
