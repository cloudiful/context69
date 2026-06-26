use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use reqwest::{Client, multipart};
use serde_json::Value;
use tokio::time::sleep;

use super::{DoclingConfig, api_base_url};

#[derive(Debug, Clone)]
pub struct DoclingXlsxClient {
    http: Client,
    base_url: String,
    poll_interval: Duration,
}

impl DoclingXlsxClient {
    pub fn new(config: DoclingConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.connection.timeout)
            .build()
            .context("failed to build docling http client")?;
        Ok(Self {
            http,
            base_url: api_base_url(&config.connection.base_url),
            poll_interval: config.connection.poll_interval,
        })
    }

    pub async fn convert_xlsx(
        &self,
        filename: &str,
        media_type: &str,
        bytes: Bytes,
    ) -> Result<Value> {
        let task_id = self.submit_async(filename, media_type, bytes).await?;

        loop {
            let status = self.poll_status(&task_id).await?;
            match status.as_str() {
                "success" => return self.fetch_result(&task_id).await,
                "failure" | "revoked" => {
                    return Err(anyhow!(
                        "docling task {task_id} failed with status {status}"
                    ));
                }
                _ => sleep(self.poll_interval).await,
            }
        }
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
            .text("to_formats", "json".to_string()))
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
        let response = response
            .error_for_status()
            .context("docling async submission returned error status")?;
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse docling async submission response")?;
        let task_id = body
            .get("task_id")
            .and_then(Value::as_str)
            .context("docling async submission missing task_id")?;
        Ok(task_id.to_string())
    }

    async fn poll_status(&self, task_id: &str) -> Result<String> {
        let response = self
            .http
            .get(format!("{}/status/poll/{task_id}", self.base_url))
            .send()
            .await
            .context("failed to poll docling status")?;
        let response = response
            .error_for_status()
            .context("docling status polling returned error status")?;
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse docling status response")?;
        body.get("task_status")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("docling status response missing task_status")
    }

    async fn fetch_result(&self, task_id: &str) -> Result<Value> {
        let response = self
            .http
            .get(format!("{}/result/{task_id}", self.base_url))
            .send()
            .await
            .context("failed to fetch docling result")?;
        let response = response
            .error_for_status()
            .context("docling result returned error status")?;
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse docling result")?;
        Ok(body
            .get("document")
            .and_then(|document| document.get("json_content"))
            .cloned()
            .or_else(|| body.get("json_content").cloned())
            .unwrap_or(body))
    }
}
