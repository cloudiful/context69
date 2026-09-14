use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts_core::errors::DomainError;
use serde_json::{Value, json};

use super::{ProviderTranslationRequest, ProviderTranslationResult, TranslationProvider};
use crate::store::StoredTranslationProvider;

pub struct LibreTranslateProvider {
    client: reqwest::Client,
    config: StoredTranslationProvider,
}

impl LibreTranslateProvider {
    pub fn new(client: reqwest::Client, config: StoredTranslationProvider) -> Self {
        Self { client, config }
    }
}

#[async_trait]
impl TranslationProvider for LibreTranslateProvider {
    async fn translate(
        &self,
        request: &ProviderTranslationRequest<'_>,
    ) -> Result<ProviderTranslationResult> {
        let endpoint = self
            .config
            .endpoint
            .as_deref()
            .ok_or_else(|| {
                DomainError::invalid_argument("LibreTranslate endpoint is not configured")
            })
            .map_err(anyhow::Error::from)?;
        let mut body = json!({
            "q": request.segments.iter().map(|segment| &segment.text).collect::<Vec<_>>(),
            "source": request.source_locale.map(language).unwrap_or("auto"),
            "target": language(request.target_locale),
            "format": "text"
        });
        if let Some(api_key) = self.config.api_key.as_deref() {
            body["api_key"] = json!(api_key);
        }
        let response = self
            .client
            .post(format!("{}/translate", endpoint.trim_end_matches('/')))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(super::provider_status_error(
                status,
                format!("LibreTranslate returned {status}: {value}"),
            )
            .into());
        }
        let values = match value.get("translatedText") {
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| value.as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>(),
            Some(Value::String(value)) => vec![value.clone()],
            _ => {
                return Err(
                    DomainError::upstream_error("LibreTranslate omitted translatedText").into(),
                );
            }
        };
        if values.len() != request.segments.len() {
            return Err(DomainError::upstream_error(
                "LibreTranslate returned an incomplete segment set",
            )
            .into());
        }
        Ok(ProviderTranslationResult {
            translations: request
                .segments
                .iter()
                .zip(values)
                .map(|(segment, value)| (segment.id.clone(), value))
                .collect::<HashMap<_, _>>(),
            model_name: self.config.endpoint.clone(),
        })
    }
}

fn language(locale: &str) -> &str {
    locale.split('-').next().unwrap_or(locale)
}
