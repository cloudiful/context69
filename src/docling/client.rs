//! The one typed HTTP client for Docling Serve.
//!
//! Every remote operation of the backend goes through this client: the
//! conversion submit, the long poll, the result fetch, and the bounded
//! management reads. It performs exactly one network attempt per operation —
//! the durable task policy owns retries, backoff, and deadlines — and it
//! never echoes an endpoint, a credential, a document byte, or a raw response
//! body into an error, a log, or a debug rendering.

use std::fmt;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use reqwest::Url;
use reqwest::multipart;

use crate::docling::DoclingConfig;
use crate::docling::error::DoclingError;
use crate::docling::models::{
    ConvertedDocument, InputFormat, PDF_BACKEND, ParsedResult, RemoteTaskSummary, SubmitRequest,
    TaskStatus, VlmSettings, code_formula_options, parse_cancel_response, parse_result,
    parse_status_response, parse_task_list, parse_task_post, parse_task_status,
    picture_description_options, vlm_pipeline_options,
};
use crate::docling::task_id::{encode_task_id_segment, validate_task_id};

/// Maximum accepted body of one submit, status, list, or cancel answer.
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;

/// Total ceiling for one remote list, cancel, or immediate-status call.
///
/// A listing either answers immediately or not at all; the conversion timeout
/// must not turn an admin refresh into a multi-second wait, and the cleanup
/// poll must not inherit the blocking client's 30s long-poll.
pub(crate) const REMOTE_OP_TIMEOUT: Duration = Duration::from_secs(5);

/// The terminal remote statuses observed via cancel and status.
///
/// Exactly the five strings the remote reports: `success`, `partial_success`,
/// `failure`, `skipped`, `cancelled`. Anything else — including `pending`,
/// `started`, and any unknown future value — is non-terminal and stays text.
fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "success" | "partial_success" | "failure" | "skipped" | "cancelled"
    )
}

/// One page of remote task identities.
#[derive(Debug, Clone)]
pub(crate) struct RemoteTaskPage {
    pub items: Vec<RemoteTaskSummary>,
    pub has_more: bool,
}

/// The outcome of one queued-cancel attempt.
///
/// A `200` that carries a recognized terminal status records that observed
/// terminal (not every terminal is literally `cancelled`). A `200` with a
/// non-terminal status and a `409` both mean the remote still holds the task:
/// await a terminal via status polling. `404` is a disappearance observation
/// only, never proof the route supports cancellation. `401`/`403` mean the
/// endpoint needs auth this deployment does not configure. Transport, `429`,
/// and `5xx` are retryable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteCancelOutcome {
    Terminal { observed_status: String },
    AwaitingTerminal,
    NotFound,
    Unsupported,
    Retryable,
    Failed,
}

/// The outcome of one immediate status read (`?wait=0`).
///
/// `200` carries the observed status text plus whether it is terminal.
/// `404` is disappearance only; `401`/`403` are unsupported; transport,
/// `429`, and `5xx` are retryable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteStatusOutcome {
    Observed {
        task_id: String,
        task_status: String,
        is_terminal: bool,
    },
    NotFound,
    Unsupported,
    Retryable,
    Failed,
}

/// The typed client for one Docling Serve endpoint.
pub(crate) struct DoclingClient {
    http: reqwest::Client,
    api_base: Url,
    timeout: Duration,
}

impl fmt::Debug for DoclingClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DoclingClient")
            .field("api_base", &"<redacted>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Validates one saved endpoint and returns its API root.
///
/// The endpoint must be an `http`/`https` URL without credentials. The path is
/// normalized to the `.../v1` API root exactly like the saved setting is, and
/// a query or fragment never becomes part of the request path.
pub(crate) fn api_base(base_url: &str) -> Result<Url, DoclingError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(DoclingError::Validation {
            parameter: "base_url".to_string(),
            reason: "the docling endpoint must not be empty".to_string(),
        });
    }
    let parsed = Url::parse(trimmed).map_err(|_| DoclingError::Validation {
        parameter: "base_url".to_string(),
        reason: "the docling endpoint must be a valid URL".to_string(),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DoclingError::Validation {
            parameter: "base_url".to_string(),
            reason: "the docling endpoint must use the http or https scheme".to_string(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DoclingError::Validation {
            parameter: "base_url".to_string(),
            reason: "the docling endpoint must not carry credentials; enter the Docling endpoint without a username or password".to_string(),
        });
    }
    let mut sanitized = parsed.clone();
    sanitized.set_query(None);
    sanitized.set_fragment(None);
    let normalized = crate::docling::api_base_url(sanitized.as_str());
    Url::parse(&normalized).map_err(|_| DoclingError::Validation {
        parameter: "base_url".to_string(),
        reason: "the docling endpoint must be a valid URL".to_string(),
    })
}

impl DoclingClient {
    /// Builds the client for the saved Docling settings.
    ///
    /// The configured connection timeout bounds one HTTP request (the async
    /// submit and every long poll), so it is clamped to the documented
    /// ceiling. The whole-document budget is `task_timeout`, which the
    /// durable task policy owns.
    pub(crate) fn new(config: &DoclingConfig) -> Result<Self, DoclingError> {
        Self::with_timeout(
            &config.connection.base_url,
            crate::docling::request_timeout(config),
        )
    }

    /// Builds the client for one explicit service root.
    ///
    /// The management operations use a short total budget so an admin refresh
    /// never waits for the conversion timeout, and a persisted job always
    /// talks to the endpoint that was captured at submit time.
    pub(crate) fn with_timeout(base_url: &str, timeout: Duration) -> Result<Self, DoclingError> {
        let api_base = api_base(base_url)?;
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .tcp_keepalive(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(30))
            // One operation is one HTTP attempt: the client never retries a
            // request on its own, and it never follows a redirect to another
            // endpoint. The durable task policy owns every retry.
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| DoclingError::Configuration {
                message: "the docling http client could not be built".to_string(),
            })?;
        Ok(Self {
            http,
            api_base,
            timeout,
        })
    }

    fn url(&self, path: &str) -> Result<Url, DoclingError> {
        let joined = format!(
            "{}/{}",
            self.api_base.as_str().trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        Url::parse(&joined).map_err(|_| DoclingError::Validation {
            parameter: "endpoint".to_string(),
            reason: "the docling endpoint must be a valid URL".to_string(),
        })
    }

    /// Submits one document as a whole-document asynchronous conversion.
    ///
    /// The returned id is the resumable remote handle; it is validated before
    /// it is persisted so a malformed identity never reaches the task row.
    pub(crate) async fn submit(&self, request: &SubmitRequest<'_>) -> Result<String, DoclingError> {
        let form = build_form(request)?;
        let url = self.url("convert/file/async")?;
        let response = self
            .http
            .post(url)
            .multipart(form)
            .send()
            .await
            .map_err(map_transport)?;
        let response = expect_success(response)?;
        let body = read_body(response, MAX_CONTROL_BODY_BYTES).await?;
        let task_id = parse_task_post(&body)?;
        validate_task_id(&task_id)?;
        Ok(task_id)
    }

    /// Polls one remote task through the long-poll endpoint.
    ///
    /// The long poll waits server-side for a bounded period; it is never the
    /// immediate `?wait=0` read.
    pub(crate) async fn poll_status(&self, task_id: &str) -> Result<TaskStatus, DoclingError> {
        validate_task_id(task_id)?;
        let url = self.url(&format!("status/poll/{}", encode_task_id_segment(task_id)))?;
        let url = with_query(url, &[("wait", "30")]);
        let response = self.http.get(url).send().await.map_err(map_transport)?;
        let response = expect_success(response)?;
        let body = read_body(response, MAX_CONTROL_BODY_BYTES).await?;
        let status = parse_task_status(&body)?;
        if status.task_id != task_id {
            return Err(DoclingError::IdentityMismatch);
        }
        Ok(status)
    }

    /// Fetches the converted document of one remote task.
    pub(crate) async fn fetch_result(
        &self,
        task_id: &str,
    ) -> Result<ConvertedDocument, DoclingError> {
        validate_task_id(task_id)?;
        let url = self.url(&format!("result/{}", encode_task_id_segment(task_id)))?;
        let response = self.http.get(url).send().await.map_err(map_transport)?;
        let response = expect_success(response)?;
        let is_archive = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("application/zip"));
        let body = read_body(response, crate::docling::MAX_DOCLING_OUTPUT_BYTES).await?;
        if is_archive || body.starts_with(b"PK\x03\x04") {
            return Err(DoclingError::Validation {
                parameter: "result".to_string(),
                reason: "the docling result was an archive, which this client does not consume"
                    .to_string(),
            });
        }
        match parse_result(&body)? {
            ParsedResult::Failed { status, message } => {
                Err(DoclingError::TaskFailed { status, message })
            }
            ParsedResult::Document {
                markdown,
                text,
                json,
            } => Ok(ConvertedDocument {
                markdown,
                text,
                json,
            }),
        }
    }

    /// Reads one bounded page of remote task identities.
    pub(crate) async fn list_tasks(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<RemoteTaskPage, DoclingError> {
        let url = self.url("tasks")?;
        let url = with_query(
            url,
            &[
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        );
        let response = self.http.get(url).send().await.map_err(map_transport)?;
        let response = expect_success(response)?;
        let body = read_body(response, MAX_CONTROL_BODY_BYTES).await?;
        let (items, has_more) = parse_task_list(&body)?;
        Ok(RemoteTaskPage { items, has_more })
    }

    /// Cancels one queued remote task.
    pub(crate) async fn cancel_task(
        &self,
        task_id: &str,
    ) -> Result<RemoteCancelOutcome, DoclingError> {
        validate_task_id(task_id)?;
        let url = self.url(&format!("tasks/{}/cancel", encode_task_id_segment(task_id)))?;
        let response = self.http.post(url).send().await.map_err(map_transport)?;
        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            401 | 403 => return Ok(RemoteCancelOutcome::Unsupported),
            404 => return Ok(RemoteCancelOutcome::NotFound),
            409 => return Ok(RemoteCancelOutcome::AwaitingTerminal),
            429 => return Ok(RemoteCancelOutcome::Retryable),
            500..=599 => return Ok(RemoteCancelOutcome::Retryable),
            _ => return Ok(RemoteCancelOutcome::Failed),
        }
        let body = read_body(response, MAX_CONTROL_BODY_BYTES).await?;
        let (answered_task_id, task_type, task_status) = parse_cancel_response(&body)?;
        if answered_task_id != task_id || task_type.as_deref().is_some_and(str::is_empty) {
            return Err(DoclingError::IdentityMismatch);
        }
        Ok(match task_status.as_deref() {
            Some("") => RemoteCancelOutcome::Failed,
            Some(status) if is_terminal_status(status) => RemoteCancelOutcome::Terminal {
                observed_status: status.to_string(),
            },
            Some(_) => RemoteCancelOutcome::AwaitingTerminal,
            None => RemoteCancelOutcome::Failed,
        })
    }

    /// Reads the immediate status of one remote task (`?wait=0`).
    pub(crate) async fn immediate_status(
        &self,
        task_id: &str,
    ) -> Result<RemoteStatusOutcome, DoclingError> {
        validate_task_id(task_id)?;
        let url = self.url(&format!("status/poll/{}", encode_task_id_segment(task_id)))?;
        let url = with_query(url, &[("wait", "0")]);
        let response = self.http.get(url).send().await.map_err(map_transport)?;
        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            401 | 403 => return Ok(RemoteStatusOutcome::Unsupported),
            404 => return Ok(RemoteStatusOutcome::NotFound),
            429 => return Ok(RemoteStatusOutcome::Retryable),
            500..=599 => return Ok(RemoteStatusOutcome::Retryable),
            _ => return Ok(RemoteStatusOutcome::Failed),
        }
        let body = read_body(response, MAX_CONTROL_BODY_BYTES).await?;
        let (answered_task_id, task_status, task_type) = parse_status_response(&body)?;
        if answered_task_id != task_id || task_type.as_deref().is_some_and(str::is_empty) {
            return Err(DoclingError::IdentityMismatch);
        }
        let Some(task_status) = task_status else {
            return Ok(RemoteStatusOutcome::Failed);
        };
        if task_status.is_empty() {
            return Ok(RemoteStatusOutcome::Failed);
        }
        Ok(RemoteStatusOutcome::Observed {
            task_id: answered_task_id,
            is_terminal: is_terminal_status(&task_status),
            task_status,
        })
    }
}

fn with_query(mut url: Url, pairs: &[(&str, &str)]) -> Url {
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(key, value);
        }
    }
    url
}

/// Builds the multipart body of one conversion submission.
fn build_form(request: &SubmitRequest<'_>) -> Result<multipart::Form, DoclingError> {
    let part = multipart::Part::stream(Bytes::clone(request.bytes))
        .file_name(request.filename.to_string())
        .mime_str(request.media_type)
        .map_err(|_| DoclingError::Validation {
            parameter: "file".to_string(),
            reason: "the document media type is not a valid multipart value".to_string(),
        })?;

    let mut form = multipart::Form::new()
        .part("files", part)
        .text("target_type", "inbody")
        .text("from_formats", request.input_format.as_from_formats());
    for format in request.output_formats {
        form = form.text("to_formats", format.as_api_value());
    }
    if request.input_format == InputFormat::Pdf {
        form = form.text("pdf_backend", PDF_BACKEND);
    }
    if let Some(preset) = request.picture_description_preset {
        form = form.text("picture_description_preset", preset.to_string());
    }
    if request.input_format.supports_vlm()
        && let Some(vlm) = request.vlm
    {
        form = apply_vlm_config(form, vlm, request.picture_description_preset.is_some())?;
    }
    Ok(form)
}

/// Adds the VLM form fields for the configured provider bundle.
///
/// A preset suppresses only the picture-description configuration; the
/// pipeline and code/formula mappings stay.
fn apply_vlm_config(
    form: multipart::Form,
    vlm: &VlmSettings,
    has_preset: bool,
) -> Result<multipart::Form, DoclingError> {
    let pipeline = serde_json::to_string(&vlm_pipeline_options(vlm)).map_err(|_| {
        DoclingError::Configuration {
            message: "the vlm pipeline configuration could not be encoded".to_string(),
        }
    })?;
    let code_formula = serde_json::to_string(&code_formula_options(vlm)).map_err(|_| {
        DoclingError::Configuration {
            message: "the vlm code and formula configuration could not be encoded".to_string(),
        }
    })?;
    let mut form = form.text("vlm_pipeline_custom_config", pipeline);
    if !has_preset {
        let picture = serde_json::to_string(&picture_description_options(vlm)).map_err(|_| {
            DoclingError::Configuration {
                message: "the vlm picture description configuration could not be encoded"
                    .to_string(),
            }
        })?;
        form = form.text("picture_description_custom_config", picture);
    }
    Ok(form
        .text("code_formula_custom_config", code_formula)
        .text("do_code_enrichment", "true")
        .text("do_formula_enrichment", "true")
        .text("do_picture_description", "true")
        .text("ocr_engine", "rapidocr")
        .text("image_export_mode", "placeholder"))
}

/// Turns a non-success answer into a typed failure without reading its body.
fn expect_success(response: reqwest::Response) -> Result<reqwest::Response, DoclingError> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(response);
    }
    Err(DoclingError::Http { status })
}

/// Reads one answer body within its bound.
///
/// Both a declared length above the bound and a streamed body that grows past
/// it are rejected, so a missing length never turns into an unbounded read.
async fn read_body(response: reqwest::Response, limit: usize) -> Result<Bytes, DoclingError> {
    if let Some(length) = response.content_length()
        && length > limit as u64
    {
        return Err(DoclingError::ResultTooLarge { limit });
    }
    let mut body = BytesMut::with_capacity(0);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| DoclingError::Transport {
            message: "the docling response could not be read".to_string(),
        })?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(DoclingError::ResultTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

/// Classifies one transport failure without echoing its message.
fn map_transport(error: reqwest::Error) -> DoclingError {
    if error.is_timeout() {
        return DoclingError::Timeout;
    }
    if error.is_builder() {
        return DoclingError::Validation {
            parameter: "request".to_string(),
            reason: "the docling request could not be built".to_string(),
        };
    }
    if error.is_connect() {
        return DoclingError::Transport {
            message: "the docling endpoint could not be reached".to_string(),
        };
    }
    if error.is_body() || error.is_decode() {
        return DoclingError::Transport {
            message: "the docling response could not be read".to_string(),
        };
    }
    DoclingError::Transport {
        message: "the docling request failed".to_string(),
    }
}
