use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use context69_contracts::{ApiErrorResponse, DocumentResponse, SearchRequest, SearchResponse};
use context69_http_support::{CurrentUser, internal_error_response, json_error_response};
use utoipa::OpenApi;

#[async_trait]
pub trait SearchApi: Send + Sync {
    async fn search(&self, user_id: Option<i64>, request: SearchRequest) -> Result<SearchResponse>;
    async fn get_document(
        &self,
        user_id: Option<i64>,
        document_id: i64,
    ) -> Result<DocumentResponse>;
}

#[derive(Clone)]
pub struct SearchHttpState {
    pub search: Arc<dyn SearchApi>,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    SearchHttpState: FromRef<S>,
{
    Router::new()
        .route("/v1/search", post(search))
        .route("/v1/documents/{document_id}", get(get_document))
}

#[derive(OpenApi)]
#[openapi(
    paths(search, get_document),
    components(schemas(ApiErrorResponse, SearchRequest, SearchResponse, DocumentResponse)),
    tags((name = "search", description = "Search transport"))
)]
struct SearchApiDoc;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    SearchApiDoc::openapi()
}

#[utoipa::path(
    post,
    path = "/v1/search",
    request_body = SearchRequest,
    responses(
        (status = 200, body = SearchResponse),
        (status = 500, body = ApiErrorResponse)
    )
)]
async fn search(
    State(state): State<SearchHttpState>,
    CurrentUser(user): CurrentUser,
    axum::Json(request): axum::Json<SearchRequest>,
) -> impl IntoResponse {
    match state.search.search(Some(user.user_id), request).await {
        Ok(response) => (StatusCode::OK, axum::Json(response)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/documents/{document_id}",
    params(("document_id" = i64, Path)),
    responses(
        (status = 200, body = DocumentResponse),
        (status = 404, body = ApiErrorResponse),
        (status = 500, body = ApiErrorResponse)
    )
)]
async fn get_document(
    State(state): State<SearchHttpState>,
    CurrentUser(user): CurrentUser,
    Path(document_id): Path<i64>,
) -> impl IntoResponse {
    match state
        .search
        .get_document(Some(user.user_id), document_id)
        .await
    {
        Ok(document) => (StatusCode::OK, axum::Json(document)).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            json_error_response(StatusCode::NOT_FOUND, error.to_string())
        }
        Err(error) => internal_error_response(error),
    }
}
