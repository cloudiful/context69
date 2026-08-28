mod llm;

use anyhow::Result;
use context69_contracts::ExtractionFailureClass;
use serde_json::Value;

use crate::store::StoredExtractionProvider;

#[derive(Debug, Clone)]
pub struct ProviderExtractionRequest<'a> {
    pub system_prompt: &'a str,
    pub user_content: &'a str,
    pub output_schema: &'a Value,
    pub max_output_tokens: i32,
}

#[derive(Debug, Clone)]
pub struct ProviderExtractionResult {
    pub result: Value,
    pub model_name: Option<String>,
}

#[derive(Debug)]
pub struct ProviderHttpError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for ProviderHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LLM provider returned {}: {}", self.status, self.body)
    }
}

impl std::error::Error for ProviderHttpError {}

#[derive(Debug)]
pub struct ProviderSchemaError(pub String);

impl std::fmt::Display for ProviderSchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderSchemaError {}

#[derive(Debug)]
pub struct ProviderPayloadError(pub String);

impl std::fmt::Display for ProviderPayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderPayloadError {}

pub async fn extract(
    client: &reqwest::Client,
    provider: &StoredExtractionProvider,
    request: &ProviderExtractionRequest<'_>,
) -> Result<ProviderExtractionResult> {
    let llm_provider = llm::LlmProvider::new(client.clone(), provider.clone());
    let result = llm_provider.extract_payload(request).await?;
    validate_schema(request.output_schema, &result.result)?;
    Ok(result)
}

pub fn classify_error(error: &anyhow::Error) -> ExtractionFailureClass {
    if let Some(http) = error.downcast_ref::<ProviderHttpError>() {
        return match http.status {
            429 => ExtractionFailureClass::QuotaExceeded,
            500..=599 => ExtractionFailureClass::Transient,
            408 => ExtractionFailureClass::Transient,
            400..=499 => ExtractionFailureClass::Permanent,
            _ => ExtractionFailureClass::Transient,
        };
    }
    if let Some(req) = error.downcast_ref::<reqwest::Error>() {
        if req.is_timeout() || req.is_connect() {
            return ExtractionFailureClass::Transient;
        }
        if let Some(status) = req.status() {
            return match status.as_u16() {
                429 => ExtractionFailureClass::QuotaExceeded,
                500..=599 => ExtractionFailureClass::Transient,
                408 => ExtractionFailureClass::Transient,
                400..=499 => ExtractionFailureClass::Permanent,
                _ => ExtractionFailureClass::Transient,
            };
        }
        return ExtractionFailureClass::Transient;
    }
    if error.downcast_ref::<ProviderSchemaError>().is_some()
        || error.downcast_ref::<ProviderPayloadError>().is_some()
    {
        return ExtractionFailureClass::Permanent;
    }
    // Walk the error chain for typed causes wrapped via anyhow context
    for cause in error.chain() {
        if let Some(http) = cause.downcast_ref::<ProviderHttpError>() {
            return match http.status {
                429 => ExtractionFailureClass::QuotaExceeded,
                500..=599 => ExtractionFailureClass::Transient,
                408 => ExtractionFailureClass::Transient,
                400..=499 => ExtractionFailureClass::Permanent,
                _ => ExtractionFailureClass::Transient,
            };
        }
        if let Some(req) = cause.downcast_ref::<reqwest::Error>() {
            if req.is_timeout() || req.is_connect() {
                return ExtractionFailureClass::Transient;
            }
            if let Some(status) = req.status() {
                return match status.as_u16() {
                    429 => ExtractionFailureClass::QuotaExceeded,
                    500..=599 => ExtractionFailureClass::Transient,
                    408 => ExtractionFailureClass::Transient,
                    400..=499 => ExtractionFailureClass::Permanent,
                    _ => ExtractionFailureClass::Transient,
                };
            }
            return ExtractionFailureClass::Transient;
        }
        if cause.downcast_ref::<ProviderSchemaError>().is_some()
            || cause.downcast_ref::<ProviderPayloadError>().is_some()
        {
            return ExtractionFailureClass::Permanent;
        }
    }
    let msg = error.to_string().to_lowercase();
    if msg.contains("output_schema") || msg.contains("violates") {
        return ExtractionFailureClass::Permanent;
    }
    if msg.contains("omitted submit_extraction") || msg.contains("llm response is not json") {
        return ExtractionFailureClass::Permanent;
    }
    if msg.contains("429")
        || msg.contains("quota_exceeded")
        || msg.contains("quota exceeded")
        || msg.contains("rate limit")
    {
        return ExtractionFailureClass::QuotaExceeded;
    }
    if msg.contains("timeout")
        || msg.contains("timed out")
        || msg.contains("connection")
        || msg.contains("transport")
    {
        return ExtractionFailureClass::Transient;
    }
    if msg.contains("401")
        || msg.contains("403")
        || msg.contains("authentication")
        || msg.contains("unauthorized")
        || msg.contains("forbidden")
    {
        return ExtractionFailureClass::Permanent;
    }
    if msg.contains(" 500") || msg.contains(" 502") || msg.contains(" 503") || msg.contains(" 504")
    {
        return ExtractionFailureClass::Transient;
    }
    ExtractionFailureClass::Permanent
}

pub fn failure_class_as_str(class: ExtractionFailureClass) -> &'static str {
    match class {
        ExtractionFailureClass::Transient => "transient",
        ExtractionFailureClass::QuotaExceeded => "quota_exceeded",
        ExtractionFailureClass::Permanent => "permanent",
    }
}

pub fn next_retry_delay(attempt_count: i32) -> std::time::Duration {
    // Bounded retry: 5s after the first execution, 10s after the second, capped at 300s.
    // With MAX_ATTEMPTS=3 (total executions), only the first two transient failures
    // schedule a retry (attempts 1 and 2); the third is terminal. The exponential
    // formula is kept for future caps but the 20s value is not scheduled under the
    // current 3-attempt budget.
    let exp = attempt_count.saturating_sub(1).max(0) as u32;
    let secs = 5u64.saturating_mul(2u64.saturating_pow(exp));
    std::time::Duration::from_secs(secs.min(300))
}

fn validate_schema(schema: &Value, instance: &Value) -> Result<()> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| ProviderSchemaError(format!("output_schema is invalid: {error}")))
        .map_err(|e| anyhow::Error::new(e))?;
    validator.validate(instance).map_err(|error| {
        anyhow::Error::new(ProviderSchemaError(format!(
            "extraction result violates output_schema: {error}"
        )))
    })?;
    Ok(())
}
