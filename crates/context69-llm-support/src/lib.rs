//! Shared transport primitives for LLM-backed providers.
//!
//! Extraction and translation keep their own request payloads, tool names,
//! prompts, schemas, token policies, result validation, and domain error
//! types. This crate only owns the duplicated wire mechanics: endpoint
//! normalization, non-empty model/key/kind validation, provider HTTP request
//! execution, successful JSON decoding, structured HTTP/JSON errors, and the
//! common OpenAI Chat/Responses plus Anthropic tool-call payload extraction.

use anyhow::{Context, Result};
use serde_json::Value;

/// Structured error for non-2xx provider HTTP responses.
#[derive(Debug)]
pub struct LlmHttpError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for LlmHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LLM provider returned {}: {}", self.status, self.body)
    }
}

impl std::error::Error for LlmHttpError {}

/// Structured error for provider responses that are not usable JSON.
#[derive(Debug)]
pub struct LlmPayloadError(pub String);

impl std::fmt::Display for LlmPayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LlmPayloadError {}

/// Append `suffix` to a configured base endpoint, tolerating a trailing `/`.
///
/// Returns the endpoint unchanged when it already ends with `suffix`, so a
/// fully qualified endpoint keeps working without producing a doubled path.
pub fn normalize_endpoint(endpoint: Option<&str>, suffix: &str) -> Result<String> {
    let endpoint = endpoint.context("LLM endpoint is not configured")?;
    if endpoint.ends_with(suffix) {
        Ok(endpoint.to_string())
    } else {
        Ok(format!("{}{}", endpoint.trim_end_matches('/'), suffix))
    }
}

/// Require a configured, non-empty provider model name.
pub fn require_model(model: Option<&str>) -> Result<&str> {
    model
        .filter(|value| !value.is_empty())
        .context("LLM model is not configured")
}

/// Require a configured, non-empty provider API key.
pub fn require_api_key(api_key: Option<&str>) -> Result<&str> {
    api_key
        .filter(|value| !value.is_empty())
        .context("LLM api_key is not configured")
}

/// Require a configured, non-empty provider API kind.
pub fn require_api_kind(api_kind: Option<&str>) -> Result<&str> {
    api_kind
        .filter(|value| !value.is_empty())
        .context("LLM api kind is not configured")
}

/// Decode a provider HTTP response into JSON, mapping non-2xx statuses to
/// [`LlmHttpError`] and undecodable bodies to [`LlmPayloadError`].
pub async fn decode_successful_json(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(anyhow::Error::new(LlmHttpError {
            status: status.as_u16(),
            body: text,
        }));
    }
    serde_json::from_str(&text).map_err(|err| {
        anyhow::Error::new(LlmPayloadError(format!("LLM response is not JSON: {err}")))
    })
}

/// Send an already-built provider request and decode the successful JSON body.
pub async fn send_and_decode(builder: reqwest::RequestBuilder) -> Result<Value> {
    let response = builder.send().await?;
    decode_successful_json(response).await
}

/// Extract the tool-call payload for a provider API kind.
///
/// Chat Completions nests stringified JSON under the first tool call,
/// Responses nests stringified JSON under the first `function_call` output,
/// and Anthropic Messages carries the parsed `input` of the first `tool_use`
/// block. Returns `None` for unknown kinds or missing payloads.
pub fn extract_tool_payload(api_kind: &str, value: &Value) -> Option<Value> {
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
    use serde_json::json;

    #[test]
    fn normalizes_endpoint_suffix() {
        assert_eq!(
            normalize_endpoint(Some("https://llm.example.com/"), "/chat/completions").unwrap(),
            "https://llm.example.com/chat/completions"
        );
        assert_eq!(
            normalize_endpoint(Some("https://llm.example.com"), "/chat/completions").unwrap(),
            "https://llm.example.com/chat/completions"
        );
        assert_eq!(
            normalize_endpoint(
                Some("https://llm.example.com/chat/completions"),
                "/chat/completions"
            )
            .unwrap(),
            "https://llm.example.com/chat/completions"
        );
    }

    #[test]
    fn rejects_missing_endpoint() {
        let err = normalize_endpoint(None, "/chat/completions").unwrap_err();
        assert!(err.to_string().contains("LLM endpoint is not configured"));
    }

    #[test]
    fn rejects_missing_model_key_and_kind() {
        assert!(require_model(None).is_err());
        assert!(require_model(Some("")).is_err());
        assert_eq!(require_model(Some("gpt-4o")).unwrap(), "gpt-4o");
        assert!(require_api_key(None).is_err());
        assert!(require_api_key(Some("")).is_err());
        assert_eq!(require_api_key(Some("secret")).unwrap(), "secret");
        assert!(require_api_kind(None).is_err());
        assert!(require_api_kind(Some("")).is_err());
        assert_eq!(
            require_api_kind(Some("openai_chat_completions")).unwrap(),
            "openai_chat_completions"
        );
    }

    #[test]
    fn extracts_chat_tool_arguments() {
        let value = json!({"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"summary\":\"标题\"}"}}]}}]});
        assert!(extract_tool_payload("openai_chat_completions", &value).is_some());
    }

    #[test]
    fn extracts_responses_function_call() {
        let value = json!({"output":[{"type":"function_call","name":"submit","arguments":"{\"summary\":\"标题\"}"}]});
        assert!(extract_tool_payload("openai_responses", &value).is_some());
    }

    #[test]
    fn extracts_anthropic_tool_input() {
        let value =
            json!({"content":[{"type":"tool_use","name":"submit","input":{"summary":"标题"}}]});
        assert!(extract_tool_payload("anthropic_messages", &value).is_some());
    }

    #[test]
    fn returns_none_for_unknown_kind_or_missing_payload() {
        assert!(extract_tool_payload("other", &json!({"choices": []})).is_none());
        assert!(extract_tool_payload("openai_chat_completions", &json!({"choices": []})).is_none());
    }
}
