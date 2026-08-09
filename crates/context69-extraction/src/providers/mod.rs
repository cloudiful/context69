mod llm;

use anyhow::{Result, anyhow};
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

fn validate_schema(schema: &Value, instance: &Value) -> Result<()> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| anyhow!("output_schema is invalid: {error}"))?;
    validator
        .validate(instance)
        .map_err(|error| anyhow!("extraction result violates output_schema: {error}"))?;
    Ok(())
}
