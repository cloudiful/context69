//! Generic forced-tool structured call transport.
//!
//! One provider-agnostic helper for callers that need a single structured
//! tool-call round trip (for example the settings connection probe): build the
//! forced-tool request for the configured API kind, execute it, extract the
//! tool payload, and validate it against the caller's JSON schema. The caller
//! keeps its own prompts, schema, tool name, token policy, and error
//! classification; this module owns only the wire mechanics.

use anyhow::Result;
use serde_json::{Value, json};

use crate::{extract_tool_payload, normalize_endpoint, send_and_decode};

/// Structured error for a payload that does not satisfy the caller's schema.
#[derive(Debug)]
pub struct LlmSchemaError(pub String);

impl std::fmt::Display for LlmSchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LlmSchemaError {}

/// One forced-tool structured call.
pub struct StructuredToolCall<'a> {
    /// Provider base endpoint; the operation suffix is appended per API kind.
    pub endpoint: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    /// One of `openai_chat_completions`, `openai_responses`, `anthropic_messages`.
    pub api_kind: &'a str,
    /// Tool name the request forces and the response is expected to carry.
    pub tool_name: &'a str,
    pub system_prompt: &'a str,
    pub user_content: &'a str,
    /// JSON Schema the returned payload must satisfy.
    pub output_schema: &'a Value,
    pub max_output_tokens: i32,
}

/// Execute one forced-tool structured call and return the validated payload.
///
/// Non-2xx responses surface as [`crate::LlmHttpError`], non-JSON or
/// tool-less responses as [`crate::LlmPayloadError`], and a payload that
/// violates `output_schema` as [`LlmSchemaError`].
pub async fn structured_tool_call(
    client: &reqwest::Client,
    call: &StructuredToolCall<'_>,
) -> Result<Value> {
    let response = match call.api_kind {
        "openai_chat_completions" => {
            let builder = client
                .post(normalize_endpoint(
                    Some(call.endpoint),
                    "/chat/completions",
                )?)
                .bearer_auth(call.api_key)
                .json(&json!({
                    "model": call.model,
                    "messages": [
                        {"role":"system", "content": call.system_prompt},
                        {"role":"user", "content": call.user_content}
                    ],
                    "tools": [{"type":"function", "function": {
                        "name": call.tool_name,
                        "description": "Return the structured result",
                        "strict": true,
                        "parameters": call.output_schema
                    }}],
                    "tool_choice": {"type":"function", "function":{"name": call.tool_name}},
                    "temperature": 0,
                    "stream": false
                }));
            send_and_decode(builder).await?
        }
        "openai_responses" => {
            let builder = client
                .post(normalize_endpoint(Some(call.endpoint), "/responses")?)
                .bearer_auth(call.api_key)
                .json(&json!({
                    "model": call.model,
                    "instructions": call.system_prompt,
                    "input": [{"role":"user", "content": call.user_content}],
                    "tools": [{
                        "type":"function",
                        "name": call.tool_name,
                        "strict": true,
                        "description":"Return the structured result",
                        "parameters": call.output_schema
                    }],
                    "tool_choice": {"type":"function", "name": call.tool_name}
                }));
            send_and_decode(builder).await?
        }
        "anthropic_messages" => {
            let builder = client
                .post(normalize_endpoint(Some(call.endpoint), "/v1/messages")?)
                .header("x-api-key", call.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": call.model,
                    "system": call.system_prompt,
                    "messages": [{"role":"user", "content": call.user_content}],
                    "tools": [{
                        "name": call.tool_name,
                        "description": "Return the structured result",
                        "input_schema": call.output_schema
                    }],
                    "tool_choice": {"type":"tool", "name": call.tool_name},
                    "max_tokens": call.max_output_tokens,
                    "temperature": 0
                }));
            send_and_decode(builder).await?
        }
        other => return Err(anyhow::anyhow!("unsupported LLM api kind {other}")),
    };

    let payload = extract_tool_payload(call.api_kind, &response).ok_or_else(|| {
        anyhow::Error::new(crate::LlmPayloadError(format!(
            "LLM response omitted {} payload",
            call.tool_name
        )))
    })?;
    validate_schema(call.output_schema, &payload)?;
    Ok(payload)
}

/// Validate `instance` against `schema`, mapping a malformed schema or a
/// violating instance to [`LlmSchemaError`].
pub fn validate_schema(schema: &Value, instance: &Value) -> Result<()> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| LlmSchemaError(format!("schema is invalid: {error}")))
        .map_err(anyhow::Error::new)?;
    validator.validate(instance).map_err(|error| {
        anyhow::Error::new(LlmSchemaError(format!(
            "structured result violates the schema: {error}"
        )))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_schema;
    use serde_json::json;

    #[test]
    fn accepts_instance_matching_schema() {
        let schema = json!({"type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}});
        assert!(validate_schema(&schema, &json!({"ok": true})).is_ok());
    }

    #[test]
    fn rejects_violating_instance_and_invalid_schema() {
        let schema = json!({"type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}});
        assert!(validate_schema(&schema, &json!({"ok": 1})).is_err());
        let invalid = json!({"type": "not-a-json-schema-type"});
        assert!(validate_schema(&invalid, &json!({})).is_err());
    }
}
