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
    let library_processing = state.app.library.processing_health().await;
    let (library_processing_ready, library_dependency_gates, library_processing_queue) =
        match library_processing {
            Ok((ready, gates, queue)) => (Some(ready), Some(gates), Some(queue)),
            Err(error) => {
                tracing::warn!(%error, "failed to read library processing dependency gates");
                (Some(false), None, None)
            }
        };
    match (db, qdrant) {
        (Ok(()), Ok(points)) => (
            StatusCode::OK,
            Json(HealthResponse {
                status: HealthStatus::Ok,
                indexed_chunks: Some(points),
                db_ok: None,
                qdrant_ok: None,
                library_processing_ready,
                library_dependency_gates,
                library_processing_queue,
            }),
        ),
        (db_result, qdrant_result) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: HealthStatus::Degraded,
                indexed_chunks: None,
                db_ok: Some(db_result.is_ok()),
                qdrant_ok: Some(qdrant_result.is_ok()),
                library_processing_ready,
                library_dependency_gates,
                library_processing_queue,
            }),
        ),
    }
}
