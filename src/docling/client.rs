use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use reqwest::{Client, Response, StatusCode, multipart};
use serde_json::Value;
use std::fmt;

use super::{DoclingConfig, api_base_url, xlsx_polling};

#[derive(Debug)]
pub(crate) struct DoclingHttpError {
    operation: String,
    status: StatusCode,
    body: String,
    source: Option<reqwest::Error>,
}

impl fmt::Display for DoclingHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} failed: HTTP {}", self.operation, self.status)?;
        if let Some(reason) = self.status.canonical_reason() {
            write!(formatter, " {reason}")?;
        }

        if self.body.trim().is_empty() {
            return Ok(());
        }

        let details = extract_error_details(&self.body);
        if details.is_empty() {
            write!(formatter, "; response body: {}", self.body)
        } else {
            write!(formatter, "; {details}; response body: {}", self.body)
        }
    }
}

impl std::error::Error for DoclingHttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

pub(crate) async fn ensure_success(response: Response, operation: &str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let source = response.error_for_status_ref().err();
    let body = response
        .text()
        .await
        .context("failed to read Docling error response body")?;
    Err(anyhow::Error::new(DoclingHttpError {
        operation: operation.to_string(),
        status,
        body,
        source,
    }))
}

fn extract_error_details(body: &str) -> String {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };

    ["error", "message", "detail"]
        .into_iter()
        .filter_map(|key| {
            json.get(key)
                .map(|value| format!("{key}: {}", format_json_value(value)))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_json_value(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string())
}

#[derive(Debug, Clone)]
pub struct DoclingXlsxClient {
    http: Client,
    base_url: String,
    poll_interval: Duration,
    task_timeout: Duration,
}

impl DoclingXlsxClient {
    pub fn new(config: DoclingConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(
                config
                    .connection
                    .timeout
                    .min(Duration::from_secs(super::DEFAULT_DOCLING_TIMEOUT_SECS)),
            )
            .tcp_keepalive(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .context("failed to build docling http client")?;
        Ok(Self {
            http,
            base_url: api_base_url(&config.connection.base_url),
            poll_interval: config.connection.poll_interval,
            task_timeout: config.connection.task_timeout,
        })
    }

    pub async fn convert_xlsx(
        &self,
        filename: &str,
        media_type: &str,
        bytes: Bytes,
    ) -> Result<Value> {
        // Do not replay submission: the server may have accepted the POST even if its response was lost.
        let task_id = self.submit_async(filename, media_type, bytes).await?;
        xlsx_polling::wait_for_result(
            &self.http,
            &self.base_url,
            &task_id,
            self.poll_interval,
            self.task_timeout,
        )
        .await
    }

    fn build_form(
        &self,
        filename: &str,
        media_type: &str,
        bytes: Bytes,
    ) -> Result<multipart::Form> {
        let part = multipart::Part::stream(reqwest::Body::from(bytes))
            .file_name(filename.to_string())
            .mime_str(media_type)
            .context("failed to build multipart file part")?;
        Ok(multipart::Form::new()
            .part("files", part)
            .text("from_formats", "xlsx".to_string())
            .text("to_formats", "json".to_string())
            .text("target_type", "inbody".to_string()))
    }

    async fn submit_async(&self, filename: &str, media_type: &str, bytes: Bytes) -> Result<String> {
        let form = self.build_form(filename, media_type, bytes)?;
        let response = self
            .http
            .post(format!("{}/convert/file/async", self.base_url))
            .multipart(form)
            .send()
            .await
            .context("failed to submit docling async job")?;
        let response = ensure_success(response, "Docling async submission").await?;
        let body = response
            .text()
            .await
            .context("failed to read docling async submission response body")?;
        let body = serde_json::from_str::<Value>(&body)
            .context("failed to parse docling async submission response")?;
        let task_id = body
            .get("task_id")
            .and_then(Value::as_str)
            .context("docling async submission missing task_id")?;
        Ok(task_id.to_string())
    }
}
