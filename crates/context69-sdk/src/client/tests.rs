use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use context69_contracts::{
    ApiErrorResponse, GroupKind, GroupResponse, LibraryFolderNode, LibraryTreeResponse,
    SearchRequest, SearchResponse, SourceOriginStatusKind, SourceStatus, Visibility,
};
use serial_test::serial;
use tokio::net::TcpListener;

use super::*;

const TEST_PAT: &str = "ctx_pat_test_token";

async fn spawn_test_server() -> String {
    let app = Router::new()
        .route("/v1/search", post(search_handler))
        .route("/v1/groups", get(list_groups_handler))
        .route("/v1/library/tree", get(library_tree_handler))
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/sources",
            get(list_project_sources_handler),
        );

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

fn sample_group() -> GroupResponse {
    let now = Utc::now();
    GroupResponse {
        group_id: 1,
        group_key: "ops".to_string(),
        parent_group_key: None,
        name: "Operations".to_string(),
        visibility: Visibility::Private,
        kind: GroupKind::Shared,
        current_role: None,
        created_at: now,
        updated_at: now,
    }
}

fn sample_library_tree() -> LibraryTreeResponse {
    LibraryTreeResponse {
        root: LibraryFolderNode {
            group_key: "".to_string(),
            project_key: "".to_string(),
            visibility: Visibility::Private,
            folder_id: None,
            parent_folder_id: None,
            name: "Library".to_string(),
            path: "/".to_string(),
            processing_count: 0,
            children: vec![],
            files: vec![],
        },
    }
}

fn sample_source_status() -> SourceStatus {
    SourceStatus {
        group_key: "ops".to_string(),
        project_key: "runbooks".to_string(),
        visibility: Visibility::Private,
        source_key: "alerts".to_string(),
        display_name: "Alerts".to_string(),
        description: Some("Operational alerts".to_string()),
        example_queries: vec!["recent paging incidents".to_string()],
        connection: "warehouse".to_string(),
        has_database_url: false,
        origin_status: SourceOriginStatusKind::Connected,
        origin_message: None,
        sync_strategy: "incremental".to_string(),
        connector_type: "postgres_sql".to_string(),
        base_query: "select * from alerts".to_string(),
        batch_size: 500,
        last_cursor_updated_at: None,
        last_cursor_external_id: None,
        last_success_at: None,
    }
}

async fn search_handler(headers: HeaderMap, Json(request): Json<SearchRequest>) -> Response {
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
    json_response(
        StatusCode::OK,
        &SearchResponse {
            query: request.query,
            hits: vec![],
        },
    )
}

async fn list_groups_handler(headers: HeaderMap) -> Response {
    if let Err(response) = require_pat(&headers) {
        return response;
    }
    json_response(StatusCode::OK, &vec![sample_group()])
}

async fn library_tree_handler(headers: HeaderMap) -> Response {
    if let Err(response) = require_pat(&headers) {
        return response;
    }
    json_response(StatusCode::OK, &sample_library_tree())
}

async fn list_project_sources_handler(
    headers: HeaderMap,
    Path((group_key, project_key)): Path<(String, String)>,
) -> Response {
    if let Err(response) = require_pat(&headers) {
        return response;
    }

    let mut source = sample_source_status();
    source.group_key = group_key;
    source.project_key = project_key;
    json_response(StatusCode::OK, &vec![source])
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
        .workspace()
        .list_groups()
        .await
        .expect_err("should require authentication");
    assert!(matches!(error, Error::AuthenticationRequired));
}

#[tokio::test]
#[serial]
async fn grouped_workspace_api_lists_groups() {
    let base_url = spawn_test_server().await;
    let client = Context69Client::builder()
        .base_url(&base_url)
        .expect("base url")
        .with_personal_access_token(TEST_PAT)
        .expect("pat")
        .build()
        .expect("client");

    let groups = client.workspace().list_groups().await.expect("list groups");

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group_key, "ops");
}

#[tokio::test]
#[serial]
async fn grouped_library_api_gets_tree() {
    let base_url = spawn_test_server().await;
    let client = Context69Client::builder()
        .base_url(&base_url)
        .expect("base url")
        .with_personal_access_token(TEST_PAT)
        .expect("pat")
        .build()
        .expect("client");

    let tree = client
        .library()
        .get_library_tree()
        .await
        .expect("get library tree");

    assert_eq!(tree.root.path, "/");
}

#[tokio::test]
#[serial]
async fn grouped_sources_api_lists_project_sources() {
    let base_url = spawn_test_server().await;
    let client = Context69Client::builder()
        .base_url(&base_url)
        .expect("base url")
        .with_personal_access_token(TEST_PAT)
        .expect("pat")
        .build()
        .expect("client");

    let sources = client
        .sources()
        .list_project_sources("ops", "runbooks")
        .await
        .expect("list project sources");

    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].project_key, "runbooks");
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
        .search()
        .search(SearchRequest {
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
        .search()
        .search(SearchRequest {
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
