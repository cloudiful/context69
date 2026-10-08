//! The Docling connectivity probe: which endpoint it dials, what it leaves
//! behind, and what it can be made to say.
//!
//! The probe runs against loopback Docling stand-ins only. Nothing here reaches
//! a real Docling Serve, submits a document, or creates or mutates a task, and
//! every value is synthetic. The cases assert the three properties a settings
//! test has to hold: it is non-persisting, it is credential-safe, and it is
//! bounded.

use std::sync::{Arc, Mutex};

use context69::contracts::UpdateDoclingConnectionSettings;
use tokio::net::TcpListener;

use super::support::{keyed, reset, run, service, store_metadata};
use context69::services::secret_store::key_names;

const SYNTHETIC_KEY: &str = "synthetic-docling-vlm-key";

/// A recorded request body is read up to this many bytes. A probe sends none, so
/// this only bounds what the stand-in is willing to hold.
const PROBE_MAX_BODY_BYTES: usize = 64 * 1024;

/// One request the stand-in Docling answered, as the probe sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    method: String,
    path: String,
    query: Option<String>,
    /// Every request header name the probe sent, lowercased.
    header_names: Vec<String>,
    body: String,
}

/// A loopback Docling stand-in that records what reached it and answers a fixed
/// status on the liveness path.
struct StandIn {
    base_url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl StandIn {
    /// The requests the stand-in received, in arrival order.
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("the Docling stand-in log").clone()
    }
}

/// A Docling stand-in answering `status` on every route and recording what
/// reached it. Answering every route keeps a probe that strayed off the liveness
/// path visible: the recorded request is what proves where it actually went.
async fn docling_stand_in(status: axum::http::StatusCode) -> StandIn {
    use axum::{Router, extract::Request, response::IntoResponse};

    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let recorded = seen.clone();
    let app = Router::new()
        .fallback(move |request: Request| {
            let seen = recorded.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, PROBE_MAX_BODY_BYTES)
                    .await
                    .unwrap_or_default();
                seen.lock().expect("the Docling stand-in log").push(Seen {
                    method: parts.method.to_string(),
                    path: parts.uri.path().to_string(),
                    query: parts.uri.query().map(str::to_string),
                    header_names: parts
                        .headers
                        .iter()
                        .map(|(name, _)| name.as_str().to_ascii_lowercase())
                        .collect(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
                // A body the probe must not read or return, whatever the status.
                (
                    status,
                    axum::Json(serde_json::json!({
                        "status": "ok",
                        "detail": "a probe must never surface this body",
                    })),
                )
                    .into_response()
            }
        })
        .with_state(());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the Docling stand-in");
    let address = listener.local_addr().expect("the stand-in address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve the Docling stand-in");
    });
    StandIn {
        base_url: format!("http://{address}"),
        seen,
    }
}

/// The submitted connection block, pointed at `base_url`.
fn connection(base_url: &str) -> UpdateDoclingConnectionSettings {
    UpdateDoclingConnectionSettings {
        base_url: base_url.to_string(),
        timeout_secs: 5,
        poll_interval_secs: 2,
        task_timeout_secs: 3600,
        max_inflight: 1,
    }
}

/// A reachable Docling endpoint is a success, and the only thing the probe asked
/// it was liveness: one GET, no body, no credential, no task.
#[test]
fn a_reachable_docling_endpoint_passes_on_one_keyless_liveness_get() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let stand_in = docling_stand_in(axum::http::StatusCode::OK).await;

        settings
            .test_docling_connection(&connection(&stand_in.base_url))
            .await
            .expect("a reachable Docling endpoint must pass the probe");

        let seen = stand_in.seen();
        assert_eq!(
            seen.len(),
            1,
            "a probe must make exactly one request: {seen:?}"
        );
        assert_eq!(
            seen[0],
            Seen {
                method: "GET".to_string(),
                path: "/health".to_string(),
                query: None,
                header_names: Vec::new(),
                body: String::new(),
            },
            "the probe must ask liveness and nothing else, and send no credential"
        );

        reset(db).await;
    });
}

/// No probe ever authenticates, whatever the operator typed.
///
/// This is the wire-level half of the credential-safety property: a submitted
/// `user:secret@host` would make `reqwest` synthesise a `Basic` `Authorization`
/// header, so the probe rejects that form before any request exists, and the
/// stand-in records that a request it does receive carries no authorization.
#[test]
fn a_docling_probe_never_authenticates_and_rejects_userinfo() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let stand_in = docling_stand_in(axum::http::StatusCode::OK).await;

        // A reachable endpoint with credentials embedded in it is refused, and
        // refused before the dial: the stand-in must see nothing at all.
        let error = settings
            .test_docling_connection(&connection(&format!(
                "http://user:synthetic-secret@{}",
                stand_in.base_url.trim_start_matches("http://")
            )))
            .await
            .expect_err("an endpoint carrying credentials must be rejected");
        let message = format!("{error:#}");
        assert!(
            message.contains("must not carry credentials"),
            "the rejection must name the credentials rule: {message}"
        );
        assert!(
            !message.contains("synthetic-secret"),
            "the rejection must not echo the submitted credential: {message}"
        );
        assert!(
            stand_in.seen().is_empty(),
            "a rejected endpoint must never be dialled: {:?}",
            stand_in.seen()
        );

        // The same endpoint without credentials is dialled, and the request that
        // arrives carries no authorization at all.
        settings
            .test_docling_connection(&connection(&stand_in.base_url))
            .await
            .expect("a reachable Docling endpoint must pass the probe");
        let seen = stand_in.seen();
        assert_eq!(seen.len(), 1, "the probe must make exactly one request");
        assert!(
            !seen[0]
                .header_names
                .iter()
                .any(|name| name == "authorization"),
            "the probe must never authenticate: {:?}",
            seen[0].header_names
        );

        reset(db).await;
    });
}

/// A probe never writes. Neither the submitted endpoint nor the submitted
/// timeout may reach the stored row, and the VLM credential the deployment
/// already holds must be neither opened nor replaced.
#[test]
fn a_docling_probe_writes_nothing_and_leaves_the_stored_endpoint_alone() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_docling_settings(&context69::contracts::UpdateDoclingSettingsRequest {
                connection: connection("http://127.0.0.1:1"),
                vlm: context69::contracts::UpdateDoclingVlmSettings {
                    openai_base_url: Some("http://127.0.0.1:1/v1".to_string()),
                    api_key: Some(SYNTHETIC_KEY.to_string()),
                    vlm_pipeline_model: Some("context69-secret-store-test".to_string()),
                    picture_description_model: Some("context69-secret-store-test".to_string()),
                    code_formula_model: Some("context69-secret-store-test".to_string()),
                    picture_description_preset: None,
                },
            })
            .await
            .expect("seed the stored Docling settings and the sealed key");

        let stand_in = docling_stand_in(axum::http::StatusCode::OK).await;
        settings
            .test_docling_connection(&connection(&stand_in.base_url))
            .await
            .expect("the stand-in answers the probe");

        // The stored row still holds the endpoint the deployment was configured
        // with, not the endpoint that was probed.
        let stored = settings
            .get_docling_settings()
            .await
            .expect("read the stored Docling settings");
        assert_eq!(
            stored.connection.base_url.as_deref(),
            Some("http://127.0.0.1:1"),
            "a probe must not overwrite the stored endpoint"
        );
        assert_eq!(stored.connection.timeout_secs, 5);
        // The sealed credential is untouched: the probe never resolved it, so it
        // is still stored under its own purpose and still reported by presence.
        assert!(stored.vlm.has_api_key);
        assert!(
            store_metadata(db.pool(), key_names::DOCLING_VLM_API_KEY)
                .await
                .is_some(),
            "a probe must not clear or replace the stored VLM key"
        );

        reset(db).await;
    });
}

/// A probe against an endpoint that refuses it reports the refusal, bounded:
/// the status is reported, and neither the submitted URL nor anything the
/// remote sent comes back.
#[test]
fn a_refused_docling_probe_reports_a_bounded_result() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        // Unreachable loopback port, with a query string the probe must drop
        // before dialling and must never echo in the failure.
        let error = settings
            .test_docling_connection(&connection("http://127.0.0.1:1?api_key=synthetic-secret"))
            .await
            .expect_err("an unreachable endpoint must fail the probe");
        let message = format!("{error:#}");
        assert!(
            !message.contains("synthetic-secret"),
            "a probe failure must not echo the submitted URL: {message}"
        );
        assert!(
            !message.contains("127.0.0.1"),
            "a probe failure must not echo the submitted host: {message}"
        );
        assert!(
            message.contains("Docling"),
            "a probe failure must name the dependency it failed to reach: {message}"
        );

        // An undialable endpoint is rejected before any request is made, so the
        // failure is a validation error rather than a transport one.
        for (base_url, expected) in [
            ("   ", "must not be empty"),
            // A bare `host:port` parses as a URL whose scheme is the host, so the
            // scheme check is what rejects it.
            ("docling:5001", "must use the http or https scheme"),
            ("/v1/convert", "must be a valid URL"),
            ("ftp://docling:5001", "must use the http or https scheme"),
            (
                "http://user:synthetic-secret@docling:5001",
                "must not carry credentials",
            ),
        ] {
            let error = settings
                .test_docling_connection(&connection(base_url))
                .await
                .expect_err("an undialable endpoint must be rejected");
            assert!(
                error.to_string().contains(expected),
                "rejecting {base_url:?} must report {expected:?}, got: {error}"
            );
        }

        reset(db).await;
    });
}

/// A refusal is bounded: the status reaches the operator so they learn why, and
/// neither the submitted URL nor the remote's response body comes with it.
#[test]
fn a_refused_docling_probe_reports_the_status_and_nothing_else() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let stand_in = docling_stand_in(axum::http::StatusCode::SERVICE_UNAVAILABLE).await;

        let error = settings
            .test_docling_connection(&connection(&stand_in.base_url))
            .await
            .expect_err("a refusing endpoint must fail the probe");
        let message = format!("{error:#}");
        // The endpoint really was reached, so the assertions below are not
        // passing because nothing answered.
        assert_eq!(stand_in.seen().len(), 1);
        assert!(
            message.contains("503"),
            "a refusal must report its status: {message}"
        );
        assert!(
            !message.contains("a probe must never surface this body"),
            "a probe failure must not echo the response body: {message}"
        );
        assert!(
            !message.contains(&stand_in.base_url),
            "a probe failure must not echo the submitted endpoint: {message}"
        );

        reset(db).await;
    });
}
