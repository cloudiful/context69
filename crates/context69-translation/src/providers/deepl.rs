use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::{
    ProviderTranslationRequest, ProviderTranslationResult, TranslationProvider, language_code,
};
use crate::store::StoredTranslationProvider;

struct TemporaryGlossary {
    id: String,
}

pub struct DeepLProvider {
    client: reqwest::Client,
    config: StoredTranslationProvider,
}

impl DeepLProvider {
    pub fn new(client: reqwest::Client, config: StoredTranslationProvider) -> Self {
        Self { client, config }
    }

    fn base_url(&self) -> &str {
        self.config.endpoint.as_deref().unwrap_or_else(|| {
            if self.config.deepl_plan.as_deref() == Some("pro") {
                "https://api.deepl.com"
            } else {
                "https://api-free.deepl.com"
            }
        })
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .filter(|value| !value.is_empty())
            .context("DeepL api_key is not configured")
    }

    async fn create_glossary(
        &self,
        request: &ProviderTranslationRequest<'_>,
    ) -> Result<Option<TemporaryGlossary>> {
        if request.glossary.is_empty() {
            return Ok(None);
        }
        let source = request
            .source_locale
            .map(language_code)
            .context("DeepL glossary requires a detected source locale")?;
        let entries = request
            .glossary
            .iter()
            .map(|entry| format!("{}\t{}", entry.source.trim(), entry.target.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        let response = self
            .client
            .post(format!("{}/v2/glossaries", self.base_url()))
            .header(
                "Authorization",
                format!("DeepL-Auth-Key {}", self.api_key()?),
            )
            .json(&json!({
                "name": format!("context69-{}", uuid::Uuid::new_v4()),
                "source_lang": source,
                "target_lang": deepl_target(request.target_locale),
                "entries": entries,
                "entries_format": "tsv"
            }))
            .send()
            .await?;
        let status = response.status();
        let value = response.json::<serde_json::Value>().await?;
        if !status.is_success() {
            return Err(anyhow!("DeepL glossary returned {status}: {value}"));
        }
        Ok(Some(TemporaryGlossary {
            id: value
                .get("glossary_id")
                .and_then(serde_json::Value::as_str)
                .context("DeepL glossary response omitted glossary_id")?
                .to_string(),
        }))
    }

    async fn delete_glossary(&self, glossary: &TemporaryGlossary) {
        let _ = self
            .client
            .delete(format!("{}/v2/glossaries/{}", self.base_url(), glossary.id))
            .header(
                "Authorization",
                format!(
                    "DeepL-Auth-Key {}",
                    self.config.api_key.as_deref().unwrap_or_default()
                ),
            )
            .send()
            .await;
    }
}

#[derive(Deserialize)]
struct DeepLResponse {
    translations: Vec<DeepLTranslation>,
}

#[derive(Deserialize)]
struct DeepLTranslation {
    text: String,
}

#[async_trait]
impl TranslationProvider for DeepLProvider {
    async fn translate(
        &self,
        request: &ProviderTranslationRequest<'_>,
    ) -> Result<ProviderTranslationResult> {
        let glossary = self.create_glossary(request).await?;
        let mut body = json!({
            "text": request.segments.iter().map(|segment| &segment.text).collect::<Vec<_>>(),
            "target_lang": deepl_target(request.target_locale),
            "preserve_formatting": true,
        });
        if let Some(source) = request.source_locale {
            body["source_lang"] = json!(language_code(source));
        }
        if let Some(glossary) = &glossary {
            body["glossary_id"] = json!(glossary.id);
        }
        let response = self
            .client
            .post(format!("{}/v2/translate", self.base_url()))
            .header(
                "Authorization",
                format!("DeepL-Auth-Key {}", self.api_key()?),
            )
            .json(&body)
            .send()
            .await;
        if let Some(glossary) = &glossary {
            self.delete_glossary(glossary).await;
        }
        let response = response?;
        let status = response.status();
        let value = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("DeepL returned {status}: {value}"));
        }
        let parsed: DeepLResponse = serde_json::from_str(&value)?;
        if parsed.translations.len() != request.segments.len() {
            return Err(anyhow!("DeepL returned an incomplete segment set"));
        }
        Ok(ProviderTranslationResult {
            translations: request
                .segments
                .iter()
                .zip(parsed.translations)
                .map(|(segment, translated)| (segment.id.clone(), translated.text))
                .collect::<HashMap<_, _>>(),
            model_name: None,
        })
    }
}

fn deepl_target(locale: &str) -> String {
    match locale.to_ascii_lowercase().as_str() {
        "zh-cn" => "ZH-HANS".to_string(),
        "zh-tw" => "ZH-HANT".to_string(),
        "en-us" => "EN-US".to_string(),
        "en-gb" => "EN-GB".to_string(),
        _ => language_code(locale),
    }
}
