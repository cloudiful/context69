use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use context69_llm_support::{
    extract_tool_payload, normalize_endpoint, require_api_key, require_api_kind, require_model,
    send_and_decode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ProviderTranslationRequest, ProviderTranslationResult, TranslationProvider};
use crate::store::StoredTranslationProvider;

pub struct LlmProvider {
    client: reqwest::Client,
    config: StoredTranslationProvider,
}

impl LlmProvider {
    pub fn new(client: reqwest::Client, config: StoredTranslationProvider) -> Self {
        Self { client, config }
    }

    fn endpoint(&self, suffix: &str) -> Result<String> {
        normalize_endpoint(self.config.endpoint.as_deref(), suffix)
    }

    fn model(&self) -> Result<&str> {
        require_model(self.config.model.as_deref())
    }

    fn api_key(&self) -> Result<&str> {
        require_api_key(self.config.api_key.as_deref())
    }

    fn api_kind(&self) -> Result<&str> {
        require_api_kind(self.config.llm_api_kind.as_deref())
    }

    async fn openai_chat(&self, request: &ProviderTranslationRequest<'_>) -> Result<Value> {
        let builder = self
            .client
            .post(self.endpoint("/chat/completions")?)
            .bearer_auth(self.api_key()?)
            .json(&json!({
                "model": self.model()?,
                "messages": messages(request),
                "tools": [{"type":"function", "function": tool_schema()}],
                "tool_choice": {"type":"function", "function":{"name":"submit_translations"}},
                "temperature": 0,
                "stream": false
            }));
        send_and_decode(builder).await
    }

    async fn openai_responses(&self, request: &ProviderTranslationRequest<'_>) -> Result<Value> {
        let builder = self
            .client
            .post(self.endpoint("/responses")?)
            .bearer_auth(self.api_key()?)
            .json(&json!({
                "model": self.model()?,
                "instructions": system_prompt(request),
                "input": [{"role":"user", "content": user_content(request)}],
                "tools": [{
                    "type":"function", "name":"submit_translations", "strict":true,
                    "description":"Return every translated segment exactly once",
                    "parameters": tool_parameters()
                }],
                "tool_choice": {"type":"function", "name":"submit_translations"}
            }));
        send_and_decode(builder).await
    }

    async fn anthropic(&self, request: &ProviderTranslationRequest<'_>) -> Result<Value> {
        let builder = self
            .client
            .post(self.endpoint("/v1/messages")?)
            .header("x-api-key", self.api_key()?)
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": self.model()?,
                "system": system_prompt(request),
                "messages": [{"role":"user", "content":user_content(request)}],
                "tools": [{
                    "name":"submit_translations",
                    "description":"Return every translated segment exactly once",
                    "input_schema":tool_parameters()
                }],
                "tool_choice":{"type":"tool", "name":"submit_translations"},
                "max_tokens": 8192,
                "temperature": 0
            }));
        send_and_decode(builder).await
    }
}

#[async_trait]
impl TranslationProvider for LlmProvider {
    async fn translate(
        &self,
        request: &ProviderTranslationRequest<'_>,
    ) -> Result<ProviderTranslationResult> {
        let api_kind = self.api_kind()?;
        let response = match api_kind {
            "openai_chat_completions" => self.openai_chat(request).await?,
            "openai_responses" => self.openai_responses(request).await?,
            "anthropic_messages" => self.anthropic(request).await?,
            other => return Err(anyhow!("unsupported LLM api kind {other}")),
        };
        let payload = extract_tool_payload(api_kind, &response)
            .context("LLM response omitted submit_translations payload")?;
        let parsed: TranslationPayload = serde_json::from_value(payload)?;
        let translations = parsed
            .segments
            .into_iter()
            .map(|segment| (segment.id, segment.text))
            .collect::<HashMap<_, _>>();
        Ok(ProviderTranslationResult {
            translations,
            model_name: self.config.model.clone(),
        })
    }
}

#[derive(Deserialize)]
struct TranslationPayload {
    segments: Vec<TranslatedSegment>,
}

#[derive(Deserialize)]
struct TranslatedSegment {
    id: String,
    text: String,
}

fn messages(request: &ProviderTranslationRequest<'_>) -> Value {
    json!([
        {"role":"system", "content":system_prompt(request)},
        {"role":"user", "content":user_content(request)}
    ])
}

fn system_prompt(request: &ProviderTranslationRequest<'_>) -> String {
    let glossary = request
        .glossary
        .iter()
        .map(|entry| format!("{} => {}", entry.source, entry.target))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Translate every segment into {}. Preserve facts, names, tickers, numbers, dates, currencies, Markdown and segment IDs. Do not summarize, omit, merge, split or add commentary. Return every input ID exactly once. Required terminology:\n{}",
        request.target_locale, glossary
    )
}

fn user_content(request: &ProviderTranslationRequest<'_>) -> String {
    serde_json::to_string(
        &request
            .segments
            .iter()
            .map(|segment| json!({"id":segment.id, "text":segment.text}))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
}

fn tool_schema() -> Value {
    json!({
        "name":"submit_translations",
        "description":"Return every translated segment exactly once",
        "strict":true,
        "parameters":tool_parameters()
    })
}

fn tool_parameters() -> Value {
    json!({
        "type":"object",
        "properties":{"segments":{"type":"array", "items":{
            "type":"object",
            "properties":{"id":{"type":"string"}, "text":{"type":"string"}},
            "required":["id", "text"],
            "additionalProperties":false
        }}},
        "required":["segments"],
        "additionalProperties":false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_chat_tool_arguments() {
        let value = json!({"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"segments\":[{\"id\":\"title\",\"text\":\"标题\"}]}"}}]}}]});
        assert!(extract_tool_payload("openai_chat_completions", &value).is_some());
    }
}
