use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use super::{
    ProviderExtractionRequest, ProviderExtractionResult, ProviderHttpError, ProviderPayloadError,
};
use crate::store::StoredExtractionProvider;

const TOOL_NAME: &str = "submit_extraction";

pub struct LlmProvider {
    client: reqwest::Client,
    config: StoredExtractionProvider,
}

impl LlmProvider {
    pub fn new(client: reqwest::Client, config: StoredExtractionProvider) -> Self {
        Self { client, config }
    }

    fn endpoint(&self, suffix: &str) -> Result<String> {
        let endpoint = self
            .config
            .endpoint
            .as_deref()
            .context("LLM endpoint is not configured")?;
        if endpoint.ends_with(suffix) {
            Ok(endpoint.to_string())
        } else {
            Ok(format!("{}{}", endpoint.trim_end_matches('/'), suffix))
        }
    }

    fn model(&self) -> Result<&str> {
        self.config
            .model
            .as_deref()
            .filter(|value| !value.is_empty())
            .context("LLM model is not configured")
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .filter(|value| !value.is_empty())
            .context("LLM api_key is not configured")
    }

    pub(crate) async fn openai_chat(
        &self,
        request: &ProviderExtractionRequest<'_>,
    ) -> Result<Value> {
        let response = self
            .client
            .post(self.endpoint("/chat/completions")?)
            .bearer_auth(self.api_key()?)
            .json(&json!({
                "model": self.model()?,
                "messages": [
                    {"role":"system", "content": request.system_prompt},
                    {"role":"user", "content": request.user_content}
                ],
                "tools": [{"type":"function", "function": tool_schema(request)}],
                "tool_choice": {"type":"function", "function":{"name": TOOL_NAME}},
                "temperature": 0,
                "stream": false
            }))
            .send()
            .await?;
        parse_http_response(response).await
    }

    pub(crate) async fn openai_responses(
        &self,
        request: &ProviderExtractionRequest<'_>,
    ) -> Result<Value> {
        let response = self
            .client
            .post(self.endpoint("/responses")?)
            .bearer_auth(self.api_key()?)
            .json(&json!({
                "model": self.model()?,
                "instructions": request.system_prompt,
                "input": [{"role":"user", "content": request.user_content}],
                "tools": [{
                    "type":"function", "name": TOOL_NAME, "strict": true,
                    "description":"Return the structured extraction result",
                    "parameters": request.output_schema
                }],
                "tool_choice": {"type":"function", "name": TOOL_NAME}
            }))
            .send()
            .await?;
        parse_http_response(response).await
    }

    pub(crate) async fn anthropic(&self, request: &ProviderExtractionRequest<'_>) -> Result<Value> {
        let response = self
            .client
            .post(self.endpoint("/v1/messages")?)
            .header("x-api-key", self.api_key()?)
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": self.model()?,
                "system": request.system_prompt,
                "messages": [{"role":"user", "content": request.user_content}],
                "tools": [{
                    "name": TOOL_NAME,
                    "description": "Return the structured extraction result",
                    "input_schema": request.output_schema
                }],
                "tool_choice": {"type":"tool", "name": TOOL_NAME},
                "max_tokens": request.max_output_tokens,
                "temperature": 0
            }))
            .send()
            .await?;
        parse_http_response(response).await
    }
}

impl LlmProvider {
    pub(crate) async fn extract_payload(
        &self,
        request: &ProviderExtractionRequest<'_>,
    ) -> Result<ProviderExtractionResult> {
        let api_kind = self
            .config
            .llm_api_kind
            .as_deref()
            .context("LLM api kind is not configured")?;
        let response = match api_kind {
            "openai_chat_completions" => self.openai_chat(request).await?,
            "openai_responses" => self.openai_responses(request).await?,
            "anthropic_messages" => self.anthropic(request).await?,
            other => return Err(anyhow!("unsupported LLM api kind {other}")),
        };
        let payload = extract_tool_payload(api_kind, &response).ok_or_else(|| {
            anyhow::Error::new(ProviderPayloadError(
                "LLM response omitted submit_extraction payload".to_string(),
            ))
        })?;
        Ok(ProviderExtractionResult {
            result: payload,
            model_name: self.config.model.clone(),
        })
    }
}

fn tool_schema(request: &ProviderExtractionRequest<'_>) -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Return the structured extraction result",
        "strict": true,
        "parameters": request.output_schema
    })
}

async fn parse_http_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(anyhow::Error::new(ProviderHttpError {
            status: status.as_u16(),
            body: text,
        }));
    }
    serde_json::from_str(&text).map_err(|err| {
        anyhow::Error::new(ProviderPayloadError(format!(
            "LLM response is not JSON: {err}"
        )))
    })
}

fn extract_tool_payload(api_kind: &str, value: &Value) -> Option<Value> {
    match api_kind {
        "openai_chat_completions" => value
            .pointer("/choices/0/message/tool_calls/0/function/arguments")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str(text).ok()),
        "openai_responses" => value
            .get("output")?
            .as_array()?
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))?
            .get("arguments")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str(text).ok()),
        "anthropic_messages" => value
            .get("content")?
            .as_array()?
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))?
            .get("input")
            .cloned(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_chat_tool_arguments() {
        let value = json!({"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"summary\":\"标题\"}"}}]}}]});
        assert!(extract_tool_payload("openai_chat_completions", &value).is_some());
    }

    #[test]
    fn extracts_anthropic_tool_input() {
        let value = json!({"content":[{"type":"tool_use","name":"submit_extraction","input":{"summary":"标题"}}]});
        assert!(extract_tool_payload("anthropic_messages", &value).is_some());
    }
}
