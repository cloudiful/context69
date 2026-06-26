use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::contracts::{HealthResponse, HealthStatus};

use super::ApiState;

#[utoipa::path(
    get,
    path = "/healthz",
    responses(
        (status = 200, description = "API is healthy", body = HealthResponse),
        (status = 503, description = "API is degraded", body = HealthResponse)
    )
)]
pub(crate) async fn healthz(State(state): State<ApiState>) -> impl IntoResponse {
    let db = state.app.db.ping().await;
    let qdrant = state.app.sync.search_smoke_test().await;

    match (db, qdrant) {
        (Ok(()), Ok(points)) => (
            StatusCode::OK,
            Json(HealthResponse {
                status: HealthStatus::Ok,
                indexed_chunks: Some(points),
                db_ok: None,
                qdrant_ok: None,
            }),
        ),
        (db_result, qdrant_result) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: HealthStatus::Degraded,
                indexed_chunks: None,
                db_ok: Some(db_result.is_ok()),
                qdrant_ok: Some(qdrant_result.is_ok()),
            }),
        ),
    }
}
