use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use reqwest::{Client, multipart};
use serde_json::Value;

use super::{DoclingConfig, api_base_url, xlsx_polling};

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
            .timeout(config.connection.timeout)
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
        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read docling async submission response body")?;
        if !status.is_success() {
            let detail = if body.trim().is_empty() {
                status.canonical_reason().unwrap_or("empty response body")
            } else {
                body.trim()
            };
            return Err(anyhow!("HTTP {status}: {detail}"));
        }
        let body = serde_json::from_str::<Value>(&body)
            .context("failed to parse docling async submission response")?;
        let task_id = body
            .get("task_id")
            .and_then(Value::as_str)
            .context("docling async submission missing task_id")?;
        Ok(task_id.to_string())
    }
}
