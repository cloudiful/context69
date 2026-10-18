//! Process liveness and service readiness.
//!
//! `/livez` answers only whether the process is running and reads no dependency.
//! `/healthz` and `/readyz` answer whether this instance can serve core work:
//! the database answers, the current vector-runtime generation completes a real
//! read, and library processing reports ready. They return `200` only when all
//! three hold and `503` otherwise.
//!
//! Readiness is decided once, after every dependency read: a settings change or
//! rebuild that starts while the request is in flight can therefore never
//! produce a stale `200`.

use std::{future::Future, time::Duration};

use anyhow::Result;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{
    contracts::{
        HealthResponse, HealthStatus, LibraryDependencyGateResponse, LibraryProcessingQueueHealth,
    },
    vector_runtime::VectorRuntime,
};

use super::ApiState;

/// Bound on the readiness vector probe, so an unresponsive remote is reported
/// unavailable instead of holding the route open.
const VECTOR_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The vector generation that answered a readiness probe.
#[derive(Clone, Copy)]
struct VectorProbe {
    generation: u64,
    points: u64,
}

// Process liveness: no database, vector, or model access.
#[utoipa::path(
    get,
    path = "/livez",
    responses(
        (status = 200, description = "Process is running", body = HealthResponse)
    )
)]
pub(crate) async fn livez() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: HealthStatus::Ok,
            indexed_chunks: None,
            db_ok: None,
            qdrant_ok: None,
            library_processing_ready: None,
            library_dependency_gates: None,
            library_processing_queue: None,
        }),
    )
}

// `/healthz` keeps readiness; `/readyz` exposes the same answer at its own path.
#[utoipa::path(
    get,
    path = "/healthz",
    responses(
        (status = 200, description = "API is healthy", body = HealthResponse),
        (status = 503, description = "API is degraded", body = HealthResponse)
    )
)]
pub(crate) async fn healthz(State(state): State<ApiState>) -> impl IntoResponse {
    readiness_response(state).await
}

// Readiness at its own path.
#[utoipa::path(
    get,
    path = "/readyz",
    responses(
        (status = 200, description = "API can serve core work", body = HealthResponse),
        (status = 503, description = "A required dependency is not ready", body = HealthResponse)
    )
)]
pub(crate) async fn readyz(State(state): State<ApiState>) -> impl IntoResponse {
    readiness_response(state).await
}

async fn readiness_response(state: ApiState) -> (StatusCode, Json<HealthResponse>) {
    let db_ok = state.app.db.ping().await.is_ok();
    let probe = probe_current_vector_generation(&state).await;
    readiness_answer(
        &state.app.vector_runtime,
        db_ok,
        probe,
        state.app.library.processing_health(),
    )
    .await
}

/// Assembles the readiness answer from already-read dependencies. The vector
/// probe is only usable when the final coherent runtime read still reports that
/// whole generation as the ready pair.
async fn readiness_answer(
    runtime: &VectorRuntime,
    db_ok: bool,
    probe: Option<VectorProbe>,
    library_processing: impl Future<
        Output = Result<(
            bool,
            Vec<LibraryDependencyGateResponse>,
            LibraryProcessingQueueHealth,
        )>,
    >,
) -> (StatusCode, Json<HealthResponse>) {
    let library_processing = library_processing.await;
    let (library_processing_ready, library_dependency_gates, library_processing_queue) =
        match library_processing {
            Ok((ready, gates, queue)) => (Some(ready), Some(gates), Some(queue)),
            Err(error) => {
                tracing::warn!(%error, "failed to read library processing dependency gates");
                (Some(false), None, None)
            }
        };

    let current = runtime.current();
    let vector_answered = probe.is_some();
    let indexed_chunks = probe
        .filter(|probe| {
            current.ready
                && probe.generation == current.generation
                && current
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.generation == probe.generation)
        })
        .map(|probe| probe.points);

    if readiness_verdict(db_ok, indexed_chunks, library_processing_ready) {
        let chunks = indexed_chunks.expect("a ready verdict requires a current vector probe");
        (
            StatusCode::OK,
            Json(HealthResponse {
                status: HealthStatus::Ok,
                indexed_chunks: Some(chunks),
                db_ok: None,
                qdrant_ok: None,
                library_processing_ready,
                library_dependency_gates,
                library_processing_queue,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: HealthStatus::Degraded,
                indexed_chunks: None,
                db_ok: Some(db_ok),
                qdrant_ok: Some(vector_answered),
                library_processing_ready,
                library_dependency_gates,
                library_processing_queue,
            }),
        )
    }
}

/// Every required dependency must actually be serving, not merely configured: a
/// processing gate that is not ready fails readiness even when the database and
/// the vector store individually answer.
fn readiness_verdict(
    db_ok: bool,
    indexed_chunks: Option<u64>,
    library_processing_ready: Option<bool>,
) -> bool {
    db_ok && indexed_chunks.is_some() && library_processing_ready == Some(true)
}

/// Reads the current pair and performs one bounded, read-only probe of it. The
/// generation is carried to the final decision instead of being validated here,
/// because a config change or rebuild may still occur before the answer.
async fn probe_current_vector_generation(state: &ApiState) -> Option<VectorProbe> {
    let current = state.app.vector_runtime.current();
    let snapshot = current.snapshot?;
    let points = tokio::time::timeout(VECTOR_PROBE_TIMEOUT, snapshot.index.count_points())
        .await
        .ok()?
        .ok()?;
    Some(VectorProbe {
        generation: current.generation,
        points,
    })
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::{
        contracts::{LibraryDependencyGateResponse, LibraryProcessingQueueHealth},
        embedding::EmbeddingIdentity,
        qdrant_index::QdrantIndex,
        vector_runtime::VectorRuntime,
    };

    use super::{VectorProbe, readiness_answer, readiness_verdict};

    fn test_runtime() -> VectorRuntime {
        VectorRuntime::for_test_with_index(
            QdrantIndex::for_test("test", 8),
            EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
        )
    }

    fn queue_health() -> LibraryProcessingQueueHealth {
        LibraryProcessingQueueHealth {
            pending_count: 0,
            queued_count: 0,
            oldest_pending_age_seconds: None,
            oldest_queued_age_seconds: None,
            oldest_waiting_age_seconds: None,
            recent_failure_count: 0,
            docling_dependency_waiting_count: 0,
            stale_waiting_count: 0,
            status_counts: Vec::new(),
            stage_counts: Vec::new(),
            waiting_reason_counts: Vec::new(),
            dependency_counts: Vec::new(),
            processed_last_hour: 0,
            failed_last_hour: 0,
            processing_rate_per_minute: 0.0,
            failure_rate_percent: 0.0,
            parent_consistency: None,
        }
    }

    fn processing_ready() -> anyhow::Result<(
        bool,
        Vec<LibraryDependencyGateResponse>,
        LibraryProcessingQueueHealth,
    )> {
        Ok((true, Vec::new(), queue_health()))
    }

    #[test]
    fn readiness_requires_database_vector_and_processing_together() {
        assert!(readiness_verdict(true, Some(12), Some(true)));
    }

    #[test]
    fn each_missing_dependency_is_on_its_own_enough_to_fail_readiness() {
        assert!(!readiness_verdict(false, Some(12), Some(true)));
        assert!(!readiness_verdict(true, None, Some(true)));
        assert!(!readiness_verdict(true, Some(12), Some(false)));
        assert!(!readiness_verdict(true, Some(12), None));
    }

    #[tokio::test]
    async fn a_generation_change_during_the_processing_read_cannot_answer_200() {
        let runtime = test_runtime();
        let generation = runtime.generation();
        let probe = Some(VectorProbe {
            generation,
            points: 12,
        });
        let racing = runtime.clone();
        let (status, body) = readiness_answer(&runtime, true, probe, async move {
            racing.begin_generation();
            processing_ready()
        })
        .await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0.indexed_chunks, None);
        assert_eq!(body.0.qdrant_ok, Some(true));
        assert_eq!(body.0.library_processing_ready, Some(true));
    }

    #[tokio::test]
    async fn a_rebuild_started_during_the_processing_read_cannot_answer_200() {
        let runtime = test_runtime();
        let generation = runtime.generation();
        let probe = Some(VectorProbe {
            generation,
            points: 12,
        });
        let racing = runtime.clone();
        let (status, body) = readiness_answer(&runtime, true, probe, async move {
            racing.begin_rebuild();
            processing_ready()
        })
        .await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0.library_processing_ready, Some(true));
    }

    #[tokio::test]
    async fn a_probe_from_a_superseded_generation_cannot_answer_200() {
        let runtime = test_runtime();
        let stale = runtime.generation();
        runtime.begin_generation();
        let probe = Some(VectorProbe {
            generation: stale,
            points: 12,
        });

        let (status, _) =
            readiness_answer(&runtime, true, probe, async { processing_ready() }).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_current_ready_generation_and_ready_processing_answer_200() {
        let runtime = test_runtime();
        let probe = Some(VectorProbe {
            generation: runtime.generation(),
            points: 12,
        });

        let (status, body) =
            readiness_answer(&runtime, true, probe, async { processing_ready() }).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0.indexed_chunks, Some(12));
    }

    #[tokio::test]
    async fn a_missing_vector_probe_is_not_ready() {
        let runtime = test_runtime();
        let (status, body) =
            readiness_answer(&runtime, true, None, async { processing_ready() }).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0.qdrant_ok, Some(false));
    }
}
