use anyhow::{Result, anyhow};
use context69_llm_support::{
    LlmHttpError, LlmPayloadError, extract_tool_payload, normalize_endpoint, require_api_key,
    require_api_kind, require_model, send_and_decode,
};
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

    pub(crate) async fn openai_chat(
        &self,
        request: &ProviderExtractionRequest<'_>,
    ) -> Result<Value> {
        let builder = self
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
            }));
        execute(builder).await
    }

    pub(crate) async fn openai_responses(
        &self,
        request: &ProviderExtractionRequest<'_>,
    ) -> Result<Value> {
        let builder = self
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
            }));
        execute(builder).await
    }

    pub(crate) async fn anthropic(&self, request: &ProviderExtractionRequest<'_>) -> Result<Value> {
        let builder = self
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
            }));
        execute(builder).await
    }
}

impl LlmProvider {
    pub(crate) async fn extract_payload(
        &self,
        request: &ProviderExtractionRequest<'_>,
    ) -> Result<ProviderExtractionResult> {
        let api_kind = self.api_kind()?;
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

async fn execute(builder: reqwest::RequestBuilder) -> Result<Value> {
    send_and_decode(builder).await.map_err(map_support_error)
}

fn map_support_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(http) = error.downcast_ref::<LlmHttpError>() {
        return anyhow::Error::new(ProviderHttpError {
            status: http.status,
            body: http.body.clone(),
        });
    }
    if let Some(payload) = error.downcast_ref::<LlmPayloadError>() {
        return anyhow::Error::new(ProviderPayloadError(payload.0.clone()));
    }
    error
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
