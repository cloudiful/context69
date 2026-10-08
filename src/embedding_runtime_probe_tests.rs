//! Tests for the settings connection probe's provider: field validation rejects
//! an unusable configuration before any provider is built, and the built
//! provider embeds against the submitted endpoint without carrying the key into
//! an error.

use super::build_probe_provider;

async fn spawn_embedding_mock(status: axum::http::StatusCode) -> String {
    use axum::{Router, routing::post};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind embedding probe mock");
    let address = listener.local_addr().expect("mock address");
    let app = Router::new()
        .route("/embeddings", post(mock_embeddings))
        .with_state(status);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    format!("http://{address}")
}

async fn mock_embeddings(
    axum::extract::State(status): axum::extract::State<axum::http::StatusCode>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if status.is_success() {
        (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({"data": [{"embedding": [0.0, 0.0]}]})),
        )
            .into_response()
    } else {
        (status, "provider refused").into_response()
    }
}

#[tokio::test]
async fn the_probe_provider_embeds_against_a_live_endpoint() {
    let base = spawn_embedding_mock(axum::http::StatusCode::OK).await;
    let provider = build_probe_provider(
        base.as_str(),
        "probe-model",
        2,
        5,
        Some("probe-secret".to_string()),
    )
    .expect("valid probe config builds");
    let vectors = provider
        .embed_texts(&["probe".to_string()])
        .await
        .expect("the mock returns a vector");
    assert_eq!(vectors[0].len(), 2);
}

#[tokio::test]
async fn a_failed_probe_reports_the_failure_without_the_key() {
    let base = spawn_embedding_mock(axum::http::StatusCode::BAD_REQUEST).await;
    let provider = build_probe_provider(
        base.as_str(),
        "probe-model",
        2,
        5,
        Some("do-not-leak-this".to_string()),
    )
    .expect("valid probe config builds");
    let error = provider
        .embed_texts(&["probe".to_string()])
        .await
        .expect_err("the mock refuses the request");
    let message = error.to_string();
    assert!(!message.contains("do-not-leak-this"), "{message}");
}

#[test]
fn probe_validation_rejects_invalid_fields_without_the_key() {
    let key = "do-not-leak-this".to_string();
    let rejected = [
        build_probe_provider("   ", "model", 1, 5, Some(key.clone())),
        build_probe_provider("http://example", " ", 1, 5, Some(key.clone())),
        build_probe_provider("http://example", "model", 0, 5, Some(key.clone())),
        build_probe_provider("http://example", "model", 1, 0, Some(key.clone())),
    ];
    for result in rejected {
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("an invalid probe config must be rejected"),
        };
        let message = error.to_string();
        assert!(!message.contains(&key), "{message}");
    }
}
