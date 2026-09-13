use std::convert::Infallible;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use context69_contracts::search::{
    SearchPagination, SearchRequest, SearchResponse, SearchSort, SearchStreamDone,
    SearchStreamEvent, SearchStreamPage,
};
use context69_contracts::{
    ApiErrorCode, ApiErrorResponse, CanonicalSearchRequest, DocumentResponse,
};
use context69_http_support::{
    CurrentUser, json_error_response, map_document_lookup_error,
    map_search_service_error,
};
use context69_search::{AbortSignal, abort_pair};
use tokio::sync::mpsc;
use utoipa::OpenApi;

#[async_trait]
pub trait SearchApi: Send + Sync {
    async fn search(&self, user_id: Option<i64>, request: SearchRequest) -> Result<SearchResponse>;
    /// Run the staged search pipeline for one SSE connection. `abort` resolves
    /// when the SSE body is dropped (client disconnect); the pipeline selects
    /// on it next to its long-running awaits so the in-flight embed/Qdrant/
    /// keyword/rerank work stops instead of running to completion.
    async fn stream_search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
        tx: mpsc::Sender<SearchStreamEvent>,
        abort: AbortSignal,
    ) -> Result<()>;
    async fn get_document(
        &self,
        user_id: Option<i64>,
        document_id: i64,
        locale: Option<String>,
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
        .route("/v1/search/stream", get(search_stream))
        .route("/v1/documents/{document_id}", get(get_document))
}

#[derive(OpenApi)]
#[openapi(
    paths(search, search_stream, get_document),
    components(schemas(
        ApiErrorResponse,
        ApiErrorCode,
        SearchRequest,
        CanonicalSearchRequest,
        SearchResponse,
        SearchPagination,
        SearchSort,
        SearchStreamPage,
        SearchStreamDone,
        DocumentResponse,
    )),
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
        (
            status = 400,
            body = ApiErrorResponse,
            description = "Malformed or cross-ordering cursor, `page > 1` with `sort=date` (date mode is forward-only keyset pagination; pass the cursor returned in `next_cursor` to fetch the next page), or a blank query with `sort=date` (date mode matches the query against hydrated hits and never serves a 'latest N' browse)."
        ),
        (status = 500, body = ApiErrorResponse)
    )
)]
async fn search(
    State(state): State<SearchHttpState>,
    CurrentUser(user): CurrentUser,
    axum::Json(request): axum::Json<SearchRequest>,
) -> impl IntoResponse {
    // F6: validate the request up front so an invalid page/limit/cursor
    // surfaces as a clean 400 instead of falling through to the service
    // and being reported as a 500.
    if let Err(error) = validate_post_request(&request) {
        return json_error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    match state.search.search(Some(user.user_id), request).await {
        Ok(response) => (StatusCode::OK, axum::Json(response)).into_response(),
        Err(error) => map_search_service_error(error),
    }
}

/// Validate a `SearchRequest` before it reaches the search service. Pages
/// outside the supported window, page sizes outside 1..=100, malformed
/// cursors, and a blank query in `sort=date` mode are all rejected as 400.
/// Date mode is forward-only keyset pagination: `page > 1` is rejected (the
/// client must use the cursor returned in `next_cursor` to fetch the next
/// page). A blank query in date mode is rejected because the date pipeline
/// matches the query text against hydrated hits and never silently serves a
/// "latest N" browse. Database / upstream failures keep their 500 meaning.
fn validate_post_request(request: &SearchRequest) -> Result<()> {
    if request.limit == 0 || request.limit > 100 {
        return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
    }
    if request.page == 0 {
        return Err(anyhow::anyhow!("page must be greater than 0"));
    }
    if request.sort == SearchSort::Date && request.page > 1 && request.cursor.is_none() {
        // Date mode is forward-only keyset pagination: the legacy
        // `(page - 1) * limit` mapping has no meaning here and silently
        // serving page 1 would be a worse failure than a clean 400.
        return Err(anyhow::anyhow!(
            "page > 1 is not supported for sort=date; pass the cursor returned in `next_cursor` to fetch the next page"
        ));
    }
    if request.sort == SearchSort::Date && request.query.trim().is_empty() {
        // Date mode is "matching query, latest-first, no rerank": a
        // blank query would silently turn the request into a browse
        // and violate the contract. Reject as 400 before any work.
        return Err(anyhow::anyhow!(
            "query text is required for sort=date; the date pipeline matches the query against hydrated hits (title + chunk_text) and never browses the index"
        ));
    }
    if let Some(raw) = request.cursor.as_deref() {
        context69_search::decode_search_cursor_for_validation(raw)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_stream_defaults_match_legacy_wire() {
        let decoded: CanonicalSearchRequest = serde_json::from_value(serde_json::json!({
            "query": "hello"
        }))
        .expect("stream decodes with canonical defaults");
        assert_eq!(decoded.limit, 8);
        assert!(validate_canonical_stream(&decoded).is_ok());
        let request: SearchRequest = decoded.into();
        assert_eq!(request.limit, 8);
        assert_eq!(request.page, 1);
    }

    #[test]
    fn canonical_stream_rejects_zero_limit_and_date_blank_query() {
        let bad_limit: CanonicalSearchRequest = serde_json::from_value(serde_json::json!({
            "query": "hello",
            "limit": 0
        }))
        .expect("decodes zero limit");
        assert!(validate_canonical_stream(&bad_limit).is_err());

        let date_blank: CanonicalSearchRequest = serde_json::from_value(serde_json::json!({
            "query": "   ",
            "limit": 8,
            "sort": "date"
        }))
        .expect("decodes date blank");
        assert!(validate_canonical_stream(&date_blank).is_err());
    }

    #[test]
    fn canonical_stream_decodes_explicit_url_limit() {
        let uri = axum::http::Uri::from_static("/v1/search/stream?query=hello&limit=8");
        let axum::extract::Query(decoded) =
            axum::extract::Query::<CanonicalSearchRequest>::try_from_uri(&uri)
                .expect("url query decodes");
        assert_eq!(decoded.limit, 8);
        assert!(validate_canonical_stream(&decoded).is_ok());
    }

    #[test]
    fn search_service_validation_maps_to_bad_request() {
        let response =
            map_search_service_error(anyhow::anyhow!("page_size must be between 1 and 100"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let lookup = map_document_lookup_error(anyhow::anyhow!("document 42 not found"));
        assert_eq!(lookup.status(), StatusCode::NOT_FOUND);
    }
}

#[utoipa::path(
    get,
    path = "/v1/search/stream",
    params(
        ("query" = String, Query, description = "The search query text."),
        ("locale" = Option<String>, Query),
        ("limit" = Option<u32>, Query, minimum = 1, maximum = 100, description = "Opaque cursor page size; defaults to 8 via the canonical search kernel."),
        ("source_key" = Option<String>, Query),
        ("group_path" = Option<String>, Query),
        ("published_after" = Option<chrono::DateTime<chrono::Utc>>, Query, description = "RFC3339 lower bound"),
        ("published_before" = Option<chrono::DateTime<chrono::Utc>>, Query, description = "RFC3339 upper bound"),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor; only valid inside the ordering epoch that issued it."),
        ("sort" = Option<SearchSort>, Query, description = "Additive ordering mode. Defaults to `relevance`; `date` switches to latest-first without rerank.")
    ),
    responses(
        (
            status = 200,
            description = "Server-Sent Events stream (text/event-stream). Frames: `local` (stage-1 local ordering page of the requested window, emitted immediately), optional `reranked` (final ordering of the same window after the rerank stage), `done` (with `rerank_applied`), or `error` (message). Rerank cache hits still emit both page events, with the second following immediately."
        ),
        (
            status = 400,
            body = ApiErrorResponse,
            description = "Invalid query parameters or cursor, or a blank query with `sort=date` (date mode matches the query against hydrated hits and never serves a 'latest N' browse). The SSE body never starts; the 400 is returned directly."
        )
    )
)]
async fn search_stream(
    State(state): State<SearchHttpState>,
    CurrentUser(user): CurrentUser,
    Query(decoded): Query<CanonicalSearchRequest>,
) -> impl IntoResponse {
    // F6: validate the canonical query BEFORE opening the SSE response so
    // an invalid limit/cursor returns 400 instead of starting a 200
    // stream that immediately errors out.
    if let Err(error) = validate_canonical_stream(&decoded) {
        return json_error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    let (tx, rx) = mpsc::channel::<SearchStreamEvent>(8);
    let request: SearchRequest = decoded.into();
    let search = state.search.clone();
    let user_id = user.user_id;
    // F1: tie the in-flight work to the SSE response body. The producer
    // selects on `abort.wait()` next to its long-running awaits, so a client
    // disconnect (which drops the unfold state and therefore the
    // `AbortOnDrop`) aborts the embed/Qdrant/keyword/rerank work instead
    // of letting it run to completion.
    let (signal, on_drop) = abort_pair();
    tokio::spawn(async move {
        let _ = search
            .stream_search(Some(user_id), request, tx, signal)
            .await;
    });
    // The `AbortOnDrop` lives inside the unfold state. When the response
    // body is dropped (client disconnect or handler cancel) the unfold
    // state goes with it, firing the abort signal and stopping the
    // producer.
    let stream = futures::stream::unfold((rx, on_drop), |(mut rx, abort)| async move {
        rx.recv()
            .await
            .map(|event| (Ok::<_, Infallible>(sse_event(event)), (rx, abort)))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Validate the canonical stream query before opening the 200 stream.
/// Invalid limit/cursor and a blank query in `sort=date` mode surface as 400;
/// the SSE body never starts. Shares limits and cursor validation with the
/// canonical [`CanonicalSearchRequest`] kernel; legacy `page` is not part of
/// the v0.16 query surface.
fn validate_canonical_stream(query: &CanonicalSearchRequest) -> Result<()> {
    if query.limit == 0 || query.limit > 100 {
        return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
    }
    context69_http_support::validate_cursor_limit(query.limit)?;
    if query.sort == context69_contracts::SearchSort::Date && query.query.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "query text is required for sort=date; the date pipeline matches the query against hydrated hits (title + chunk_text) and never browses the index"
        ));
    }
    if let Some(raw) = query.cursor.as_deref() {
        // Reuse the cursor decoder to reject malformed/oversized payloads.
        context69_search::decode_search_cursor_for_validation(raw)?;
    }
    Ok(())
}

fn sse_event(event: SearchStreamEvent) -> Event {
    match event {
        SearchStreamEvent::Local(page) => named_json("local", &page),
        SearchStreamEvent::Reranked(page) => named_json("reranked", &page),
        SearchStreamEvent::Done(done) => named_json("done", &done),
        SearchStreamEvent::Error { message } => {
            named_json("error", &serde_json::json!({ "message": message }))
        }
    }
}

fn named_json<T: serde::Serialize>(name: &str, value: &T) -> Event {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    Event::default().event(name).data(payload)
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
    Query(query): Query<DocumentLocaleQuery>,
) -> impl IntoResponse {
    match state
        .search
        .get_document(Some(user.user_id), document_id, query.locale)
        .await
    {
        Ok(document) => (StatusCode::OK, axum::Json(document)).into_response(),
        Err(error) => map_document_lookup_error(error),
    }
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
struct DocumentLocaleQuery {
    locale: Option<String>,
}
