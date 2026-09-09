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
use context69_contracts::{ApiErrorResponse, DocumentResponse};
use context69_http_support::{CurrentUser, internal_error_response, json_error_response};
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
        SearchRequest,
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

/// GET query parameters mirroring the `SearchRequest` filters for the
/// streaming endpoint. `metadata_filters` is intentionally not exposed over a
/// GET query string; POST `/v1/search` remains the metadata-filtered surface.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct SearchStreamQuery {
    /// The search query text.
    pub query: String,
    pub locale: Option<String>,
    #[param(minimum = 1, maximum = 100)]
    pub limit: Option<usize>,
    /// DEPRECATED page number; maps to an offset cursor when `cursor` is
    /// absent. Prefer `cursor` returned in `next_cursor`/`prev_cursor`.
    #[param(minimum = 1)]
    pub page: Option<usize>,
    pub source_key: Option<String>,
    pub group_path: Option<String>,
    pub published_after: Option<chrono::DateTime<chrono::Utc>>,
    pub published_before: Option<chrono::DateTime<chrono::Utc>>,
    /// Opaque pagination cursor; only valid inside the ordering epoch that
    /// issued it.
    pub cursor: Option<String>,
    /// Additive ordering mode. Defaults to `relevance`; `date` switches the
    /// pipeline to a latest-first walk over `published_ts` windows without
    /// rerank. The cursor and the request must agree on the sort mode.
    pub sort: Option<context69_contracts::SearchSort>,
}

impl SearchStreamQuery {
    fn into_search_request(self) -> SearchRequest {
        SearchRequest {
            query: self.query,
            locale: self.locale,
            limit: self.limit.unwrap_or(8),
            page: self.page.unwrap_or(1),
            source_key: self.source_key,
            group_path: self.group_path,
            published_after: self.published_after,
            published_before: self.published_before,
            cursor: self.cursor,
            metadata_filters: Vec::new(),
            sort: self.sort.unwrap_or_default(),
        }
    }
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
        Err(error) if is_validation_error(&error) => {
            json_error_response(StatusCode::BAD_REQUEST, error.to_string())
        }
        Err(error) => internal_error_response(error),
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

/// True when the error belongs to a client-facing validation problem
/// (page/limit/cursor geometry or a blank query in date mode) and must
/// surface as a clean 400. Database or upstream failures keep their 500
/// meaning. The blank-query message mirrors `validate_post_request` so a
/// service-side `Err` (e.g. from a direct call) still surfaces as 400
/// instead of 500.
fn is_validation_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("cursor")
        || message.contains("page_size must be between 1 and 100")
        || message.contains("page must be greater than 0")
        || message.contains("page is too large")
        || message.contains("search result limit is too large")
        || (message.contains("page > 1") && message.contains("sort=date"))
        || (message.contains("query text is required") && message.contains("sort=date"))
}

#[utoipa::path(
    get,
    path = "/v1/search/stream",
    params(SearchStreamQuery),
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
    Query(query): Query<SearchStreamQuery>,
) -> impl IntoResponse {
    // F6: validate the query parameters BEFORE opening the SSE response so
    // an invalid limit/page/cursor returns 400 instead of starting a 200
    // stream that immediately errors out.
    if let Err(error) = validate_stream_query(&query) {
        return json_error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    let (tx, rx) = mpsc::channel::<SearchStreamEvent>(8);
    let request = query.into_search_request();
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

/// Validate `SearchStreamQuery` parameters before opening the 200 stream.
/// Invalid limit/page/cursor and a blank query in `sort=date` mode
/// surface as 400; the SSE body never starts. Date mode is forward-only
/// keyset pagination: `page > 1` is rejected (the client must use the
/// cursor returned in `next_cursor` to fetch the next page). A blank
/// query in date mode is rejected because the date pipeline matches the
/// query text against hydrated hits and never silently serves a
/// "latest N" browse.
fn validate_stream_query(query: &SearchStreamQuery) -> Result<()> {
    if let Some(limit) = query.limit {
        if !(1..=100).contains(&limit) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
    }
    if let Some(page) = query.page {
        if page == 0 {
            return Err(anyhow::anyhow!("page must be greater than 0"));
        }
        if query.sort == Some(context69_contracts::SearchSort::Date)
            && page > 1
            && query.cursor.is_none()
        {
            return Err(anyhow::anyhow!(
                "page > 1 is not supported for sort=date; pass the cursor returned in `next_cursor` to fetch the next page"
            ));
        }
    }
    if query.sort == Some(context69_contracts::SearchSort::Date)
        && query.query.trim().is_empty()
    {
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
        Err(error) if error.to_string().contains("not found") => {
            json_error_response(StatusCode::NOT_FOUND, error.to_string())
        }
        Err(error) => internal_error_response(error),
    }
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
struct DocumentLocaleQuery {
    locale: Option<String>,
}
