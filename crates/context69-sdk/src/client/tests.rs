use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use context69_contracts::ApiErrorResponse;
use serial_test::serial;
use tokio::net::TcpListener;

use super::*;

const TEST_PAT: &str = "ctx_pat_test_token";

async fn spawn_test_server() -> String {
    let app = Router::new().route("/v1/search", post(search_handler));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });
    format!("http://{addr}")
}

fn require_pat(headers: &HeaderMap) -> Result<(), Response> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if bearer == format!("Bearer {TEST_PAT}") {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "missing bearer token".to_string(),
            }),
        )
            .into_response())
    }
}

fn json_response<T: serde::Serialize>(status: StatusCode, value: &T) -> Response {
    (status, Json(value)).into_response()
}

async fn search_handler(
    headers: HeaderMap,
    Json(request): Json<context69_contracts::SearchRequest>,
) -> Response {
    if request.query == "unauthorized" {
        return json_response(
            StatusCode::UNAUTHORIZED,
            &ApiErrorResponse {
                error: "expired".to_string(),
            },
        );
    }
    if let Err(response) = require_pat(&headers) {
        return response;
    }
    if request.query == "bad request" {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiErrorResponse {
                error: "invalid query".to_string(),
            },
        );
    }
    StatusCode::OK.into_response()
}

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
fn parse_api_error_body() {
    let body = r#"{"error":"missing bearer token"}"#;
    assert_eq!(
        parse_api_error_message(body),
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

    let error = client
        .list_groups()
        .await
        .expect_err("should require authentication");
    assert!(matches!(error, Error::AuthenticationRequired));
}

#[tokio::test]
#[serial]
async fn unauthorized_response_does_not_refresh() {
    let base_url = spawn_test_server().await;
    let client = Context69Client::builder()
        .base_url(&base_url)
        .expect("base url")
        .with_personal_access_token(TEST_PAT)
        .expect("pat")
        .build()
        .expect("client");

    let error = client
        .search(context69_contracts::SearchRequest {
            query: "unauthorized".to_string(),
            limit: 8,
            source_key: None,
            group_key: None,
            project_key: None,
            published_after: None,
            published_before: None,
        })
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

#[tokio::test]
#[serial]
async fn surfaces_api_error_message() {
    let base_url = spawn_test_server().await;
    let client = Context69Client::builder()
        .base_url(&base_url)
        .expect("base url")
        .with_personal_access_token(TEST_PAT)
        .expect("pat")
        .build()
        .expect("client");

    let error = client
        .search(context69_contracts::SearchRequest {
            query: "bad request".to_string(),
            limit: 8,
            source_key: None,
            group_key: None,
            project_key: None,
            published_after: None,
            published_before: None,
        })
        .await
        .expect_err("should fail");

    match error {
        Error::HttpStatus {
            status, api_error, ..
        } => {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(api_error.as_deref(), Some("invalid query"));
        }
        other => panic!("unexpected error: {other}"),
    }
}
