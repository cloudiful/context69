//! Wire models of the Docling Serve protocol and the local conversions.
//!
//! The wire models stay separate from the public Context69 contracts: they
//! describe what Docling Serve sends and receives, and the local types below
//! describe what the rest of the backend consumes.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::docling::error::DoclingError;

/// The PDF backend every PDF submission requests explicitly.
pub(crate) const PDF_BACKEND: &str = "pypdfium2";

/// The input formats this client converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputFormat {
    Pdf,
    Docx,
    Xlsx,
}

impl InputFormat {
    /// The `from_formats` value Docling Serve expects for this input.
    pub(crate) fn as_from_formats(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
        }
    }

    /// Whether Docling Serve accepts the VLM options for this input format.
    ///
    /// XLSX is converted without picture description, so it never receives
    /// the VLM form fields.
    pub(crate) fn supports_vlm(self) -> bool {
        matches!(self, Self::Pdf | Self::Docx)
    }
}

/// The output formats this client requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Md,
    Text,
    Json,
}

impl OutputFormat {
    /// The `to_formats` value Docling Serve expects for this output.
    pub(crate) fn as_api_value(self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

/// The conversion state of one remote task.
///
/// The wire spelling is kept verbatim so an observation never depends on a
/// debug rendering, and an unknown future state stays readable instead of
/// failing the conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConversionStatus {
    Pending,
    Started,
    Success,
    PartialSuccess,
    Failure,
    Skipped,
    Cancelled,
    /// A state this client does not know yet.
    Unknown(String),
}

impl ConversionStatus {
    /// Reads the wire spelling of one task status.
    ///
    /// An empty status is malformed and fails; every other unknown value
    /// stays readable and non-terminal.
    pub(crate) fn parse(raw: &str) -> Result<Self, DoclingError> {
        match raw {
            "" => Err(DoclingError::Parse {
                message: "the task status is empty".to_string(),
            }),
            "pending" => Ok(Self::Pending),
            "started" => Ok(Self::Started),
            "success" => Ok(Self::Success),
            "partial_success" => Ok(Self::PartialSuccess),
            "failure" => Ok(Self::Failure),
            "skipped" => Ok(Self::Skipped),
            "cancelled" => Ok(Self::Cancelled),
            other => Ok(Self::Unknown(other.to_string())),
        }
    }

    /// The wire spelling of this state.
    pub(crate) fn wire_name(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Started => "started",
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Failure => "failure",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Unknown(raw) => raw,
        }
    }

    /// Whether the conversion reached a state it will not leave.
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Success | Self::PartialSuccess | Self::Failure | Self::Skipped | Self::Cancelled
        )
    }

    /// Whether the terminal state carries a usable result.
    pub(crate) fn is_successful(&self) -> bool {
        matches!(self, Self::Success | Self::PartialSuccess)
    }
}

/// The structured failure Docling Serve reports for a failed task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailureInfo {
    pub category: String,
    pub message: String,
    pub retryable: bool,
    pub phase: String,
}

/// One observation of a remote task status.
#[derive(Debug, Clone)]
pub(crate) struct TaskStatus {
    pub task_id: String,
    pub task_status: ConversionStatus,
    pub error_message: Option<String>,
    pub failure: Option<FailureInfo>,
}

impl TaskStatus {
    /// The message that explains a terminal unsuccessful state.
    pub(crate) fn failure_message(&self) -> Option<&str> {
        self.error_message.as_deref().or_else(|| {
            self.failure
                .as_ref()
                .map(|failure| failure.message.as_str())
        })
    }
}

/// The converted document this backend consumes.
#[derive(Debug, Clone)]
pub(crate) struct ConvertedDocument {
    pub markdown: Option<String>,
    pub text: Option<String>,
    pub json: Option<Value>,
}

/// The resolved VLM provider bundle.
///
/// The provider key belongs to the OpenAI-compatible provider, never to the
/// Docling Serve authentication header, and it never reaches a log or a
/// debug rendering.
#[derive(Clone)]
pub(crate) struct VlmSettings {
    pub openai_base_url: String,
    pub api_key: String,
    pub vlm_pipeline_model: String,
    pub picture_description_model: String,
    pub code_formula_model: String,
}

impl std::fmt::Debug for VlmSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VlmSettings")
            .field("openai_base_url", &self.openai_base_url)
            .field("api_key", &"<redacted>")
            .field("vlm_pipeline_model", &self.vlm_pipeline_model)
            .field("picture_description_model", &self.picture_description_model)
            .field("code_formula_model", &self.code_formula_model)
            .finish()
    }
}

/// The submission of one document.
#[derive(Clone)]
pub(crate) struct SubmitRequest<'a> {
    pub filename: &'a str,
    pub media_type: &'a str,
    pub bytes: &'a bytes::Bytes,
    pub input_format: InputFormat,
    pub output_formats: &'a [OutputFormat],
    pub vlm: Option<&'a VlmSettings>,
    pub picture_description_preset: Option<&'a str>,
}

impl std::fmt::Debug for SubmitRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubmitRequest")
            .field("filename", &self.filename)
            .field("media_type", &self.media_type)
            .field("bytes", &"<redacted>")
            .field("input_format", &self.input_format)
            .field("output_formats", &self.output_formats)
            .field("vlm", &self.vlm)
            .field(
                "picture_description_preset",
                &self.picture_description_preset,
            )
            .finish()
    }
}

/// One observed remote task identity.
#[derive(Debug, Clone)]
pub(crate) struct RemoteTaskSummary {
    pub task_id: String,
    pub task_type: String,
    pub task_status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// The submitted task identity.
#[derive(Debug, Deserialize)]
struct TaskPostResponse {
    pub task_id: String,
}

/// The wire shape of one task status response.
#[derive(Debug, Deserialize)]
struct TaskStatusWire {
    task_id: String,
    task_status: String,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    failure: Option<FailureInfoWire>,
}

/// The wire shape of one structured task failure.
#[derive(Debug, Deserialize)]
struct FailureInfoWire {
    category: String,
    message: String,
    retryable: bool,
    phase: String,
}

/// The wire shape of one remote task listing.
#[derive(Debug, Deserialize)]
struct ListResponseWire {
    items: Vec<ListItemWire>,
    #[serde(default)]
    has_more: bool,
}

/// The wire shape of one listed task.
#[derive(Debug, Deserialize)]
struct ListItemWire {
    task_id: String,
    task_type: String,
    task_status: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// The wire shape of one cancel answer.
#[derive(Debug, Deserialize)]
struct CancelResponseWire {
    task_id: String,
    #[serde(default)]
    task_type: Option<String>,
    #[serde(default)]
    task_status: Option<String>,
}

/// The wire shape of one immediate status answer.
#[derive(Debug, Deserialize)]
struct StatusResponseWire {
    task_id: String,
    #[serde(default)]
    task_status: Option<String>,
    #[serde(default)]
    task_type: Option<String>,
}

/// The wire shape of a failed task result.
#[derive(Debug, Deserialize)]
struct TaskFailureResultWire {
    failure: FailureInfoWire,
}

/// The wire shape of one converted document.
#[derive(Debug, Deserialize)]
struct ExportDocumentWire {
    #[serde(default)]
    md_content: Option<String>,
    #[serde(default)]
    text_content: Option<String>,
    #[serde(default)]
    json_content: Option<Value>,
}

/// The VLM engine options Docling Serve expects.
///
/// The engine carries the provider credential in `headers` (and the provider
/// endpoint in `url`). Its debug rendering therefore reports only the
/// non-sensitive engine metadata: the serialized wire value still carries the
/// real fields, but a debug render never exposes the key, the authorization
/// header, or the endpoint.
#[derive(Serialize)]
pub(crate) struct VlmEngineOptions {
    engine_type: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    concurrency: Option<i32>,
}

impl std::fmt::Debug for VlmEngineOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VlmEngineOptions")
            .field("engine_type", &self.engine_type)
            .field("url", &"<redacted>")
            .field("headers", &self.headers.as_ref().map(|_| "<redacted>"))
            .field("params", &self.params.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .field("concurrency", &self.concurrency)
            .finish()
    }
}

/// The VLM model specification Docling Serve expects.
#[derive(Debug, Serialize)]
pub(crate) struct VlmModelSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_repo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    prompt: String,
    response_format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_new_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported_engines: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_overrides: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_overrides: Option<Value>,
}

/// The picture-description VLM options Docling Serve expects.
#[derive(Debug, Serialize)]
pub(crate) struct PictureDescriptionVlmOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    picture_area_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification_allow: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification_deny: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification_min_confidence: Option<f64>,
    engine_options: VlmEngineOptions,
    generation_config: Option<Value>,
    model_spec: VlmModelSpec,
    prompt: Option<String>,
}

/// The code and formula VLM options Docling Serve expects.
#[derive(Debug, Serialize)]
pub(crate) struct CodeFormulaVlmOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extract_code: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extract_formulas: Option<bool>,
    engine_options: VlmEngineOptions,
    model_spec: VlmModelSpec,
}

/// The VLM pipeline options Docling Serve expects.
#[derive(Debug, Serialize)]
pub(crate) struct VlmPipelineOptions {
    engine_options: VlmEngineOptions,
    model_spec: VlmModelSpec,
    scale: Option<f64>,
    max_size: Option<i32>,
    batch_size: Option<i32>,
    force_backend_text: bool,
}

fn chat_completions_url(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

fn engine_options(
    base_url: &str,
    api_key: &str,
    model_name: &str,
    timeout: i32,
) -> VlmEngineOptions {
    VlmEngineOptions {
        engine_type: "api_openai".to_string(),
        url: chat_completions_url(base_url),
        headers: Some(json!({ "Authorization": format!("Bearer {api_key}") })),
        params: Some(json!({ "model": model_name })),
        timeout: Some(timeout),
        concurrency: Some(2),
    }
}

fn model_spec(model_name: &str, prompt: &str, max_new_tokens: i32) -> VlmModelSpec {
    VlmModelSpec {
        name: Some(model_name.to_owned()),
        default_repo_id: Some(model_name.to_owned()),
        revision: None,
        prompt: prompt.to_string(),
        response_format: "markdown".to_string(),
        max_new_tokens: Some(max_new_tokens),
        supported_engines: None,
        engine_overrides: None,
        api_overrides: None,
    }
}

/// The picture-description options for the configured provider.
///
/// The picture-description call keeps its own, longer provider timeout.
pub(crate) fn picture_description_options(vlm: &VlmSettings) -> PictureDescriptionVlmOptions {
    PictureDescriptionVlmOptions {
        batch_size: None,
        scale: Some(1.0),
        picture_area_threshold: None,
        classification_allow: None,
        classification_deny: None,
        classification_min_confidence: None,
        engine_options: engine_options(
            &vlm.openai_base_url,
            &vlm.api_key,
            &vlm.picture_description_model,
            60,
        ),
        generation_config: None,
        model_spec: model_spec(
            &vlm.picture_description_model,
            "Describe this image in a few sentences.",
            300,
        ),
        prompt: None,
    }
}

/// The code and formula options for the configured provider.
pub(crate) fn code_formula_options(vlm: &VlmSettings) -> CodeFormulaVlmOptions {
    CodeFormulaVlmOptions {
        scale: Some(2.0),
        max_size: None,
        extract_code: Some(true),
        extract_formulas: Some(true),
        engine_options: engine_options(
            &vlm.openai_base_url,
            &vlm.api_key,
            &vlm.code_formula_model,
            30,
        ),
        model_spec: model_spec(
            &vlm.code_formula_model,
            "Recognize code blocks and mathematical formulas in the image. For code, output the full code; for mathematical formulas, output in LaTeX format.",
            1000,
        ),
    }
}

/// The VLM pipeline options for the configured provider.
pub(crate) fn vlm_pipeline_options(vlm: &VlmSettings) -> VlmPipelineOptions {
    VlmPipelineOptions {
        engine_options: engine_options(
            &vlm.openai_base_url,
            &vlm.api_key,
            &vlm.vlm_pipeline_model,
            30,
        ),
        model_spec: model_spec(&vlm.vlm_pipeline_model, "", 1000),
        scale: Some(1.0),
        max_size: None,
        batch_size: None,
        force_backend_text: true,
    }
}

/// Reads one task status response.
pub(crate) fn parse_task_status(body: &[u8]) -> Result<TaskStatus, DoclingError> {
    let wire: TaskStatusWire = serde_json::from_slice(body).map_err(|_| DoclingError::Parse {
        message: "the task status response is not valid".to_string(),
    })?;
    let task_status = ConversionStatus::parse(&wire.task_status)?;
    Ok(TaskStatus {
        task_id: wire.task_id,
        task_status,
        error_message: wire.error_message,
        failure: wire.failure.map(|failure| FailureInfo {
            category: failure.category,
            message: failure.message,
            retryable: failure.retryable,
            phase: failure.phase,
        }),
    })
}

/// Reads one task listing.
pub(crate) fn parse_task_list(body: &[u8]) -> Result<(Vec<RemoteTaskSummary>, bool), DoclingError> {
    let wire: ListResponseWire = serde_json::from_slice(body).map_err(|_| DoclingError::Parse {
        message: "the task list is not valid".to_string(),
    })?;
    if wire.items.iter().any(|item| {
        item.task_id.is_empty() || item.task_type.is_empty() || item.task_status.is_empty()
    }) {
        return Err(DoclingError::Parse {
            message: "the task list contains a malformed entry".to_string(),
        });
    }
    let items = wire
        .items
        .into_iter()
        .map(|item| RemoteTaskSummary {
            task_id: item.task_id,
            task_type: item.task_type,
            task_status: item.task_status,
            created_at: item.created_at,
        })
        .collect();
    Ok((items, wire.has_more))
}

/// Reads one cancel answer.
pub(crate) fn parse_cancel_response(
    body: &[u8],
) -> Result<(String, Option<String>, Option<String>), DoclingError> {
    let wire: CancelResponseWire =
        serde_json::from_slice(body).map_err(|_| DoclingError::Parse {
            message: "the cancel response is not valid".to_string(),
        })?;
    Ok((wire.task_id, wire.task_type, wire.task_status))
}

/// Reads one immediate status answer.
pub(crate) fn parse_status_response(
    body: &[u8],
) -> Result<(String, Option<String>, Option<String>), DoclingError> {
    let wire: StatusResponseWire =
        serde_json::from_slice(body).map_err(|_| DoclingError::Parse {
            message: "the status response is not valid".to_string(),
        })?;
    Ok((wire.task_id, wire.task_status, wire.task_type))
}

/// Reads one submitted task identity.
pub(crate) fn parse_task_post(body: &[u8]) -> Result<String, DoclingError> {
    let wire: TaskPostResponse = serde_json::from_slice(body).map_err(|_| DoclingError::Parse {
        message: "the submission response is not valid".to_string(),
    })?;
    Ok(wire.task_id)
}

/// The parsed content of one remote task result.
#[derive(Debug)]
pub(crate) enum ParsedResult {
    /// The task failed and carries a structured failure.
    Failed { status: String, message: String },
    /// The task produced a converted document.
    Document {
        markdown: Option<String>,
        text: Option<String>,
        json: Option<Value>,
    },
}

/// The recognized content keys of one exported document.
///
/// A result is only a document when at least one of them is present. An empty
/// string or an explicit null is a legitimate empty output; an object without
/// any of them is not a document at all.
const CONTENT_KEYS: [&str; 3] = ["md_content", "text_content", "json_content"];

/// Reads one remote task result.
///
/// Only two shapes are a result: the exported document itself, and the
/// document envelope that carries it. An unrelated JSON object, an empty
/// object, and a document without any recognized content key are not results,
/// so they never become an empty success.
pub(crate) fn parse_result(body: &[u8]) -> Result<ParsedResult, DoclingError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| DoclingError::Parse {
        message: "the result response is not valid".to_string(),
    })?;

    if is_failure_result(&value) {
        let failure: TaskFailureResultWire =
            serde_json::from_value(value).map_err(|_| DoclingError::Parse {
                message: "the failure result is not valid".to_string(),
            })?;
        return Ok(ParsedResult::Failed {
            status: failure.failure.category,
            message: failure.failure.message,
        });
    }

    // The envelope state, when the answer carries one. A terminal failure
    // stays a terminal failure; a state that is not a success is never a
    // document.
    if let Some(status) = value.get("status") {
        let status = status.as_str().ok_or_else(|| DoclingError::Parse {
            message: "the result status is not a state name".to_string(),
        })?;
        match status {
            "success" | "partial_success" => {}
            "failure" | "skipped" | "cancelled" => {
                return Ok(ParsedResult::Failed {
                    status: status.to_string(),
                    message: value
                        .get("error_message")
                        .and_then(Value::as_str)
                        .unwrap_or("the remote task did not produce a document")
                        .to_string(),
                });
            }
            _ => {
                return Err(DoclingError::Parse {
                    message: "the result status is not a terminal state".to_string(),
                });
            }
        }
    }

    // The document: either the exported document itself, or the envelope that
    // carries it.
    let source = match value.get("document") {
        Some(document) if document.is_object() => document.clone(),
        Some(_) => {
            return Err(DoclingError::Parse {
                message: "the conversion result does not carry a document".to_string(),
            });
        }
        None => value.clone(),
    };

    if !CONTENT_KEYS.iter().any(|key| source.get(key).is_some()) {
        return Err(DoclingError::Parse {
            message: "the conversion result carries no document content".to_string(),
        });
    }

    let document: ExportDocumentWire =
        serde_json::from_value(source).map_err(|_| DoclingError::Parse {
            message: "the conversion result is not valid".to_string(),
        })?;
    Ok(ParsedResult::Document {
        markdown: document.md_content,
        text: document.text_content,
        json: document.json_content,
    })
}

fn is_failure_result(value: &Value) -> bool {
    value
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "TaskFailureResult")
        || value.get("failure").is_some()
}
