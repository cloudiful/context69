use anyhow::{Result, anyhow};
use context69_contracts::{
    DeeplPlan, TranslationGlossaryEntry, TranslationJobResponse, TranslationLlmApiKind,
    TranslationProviderInput, TranslationProviderKind, TranslationProviderResponse,
    TranslationStatus,
};

use super::{StoredTranslationProvider, TranslationJobRecord};

pub fn job_response(row: TranslationJobRecord) -> Result<TranslationJobResponse> {
    Ok(TranslationJobResponse {
        job_id: row.id,
        document_id: row.document_id,
        target_locale: row.target_locale,
        source_locale: row.detected_source_locale.or(row.requested_source_locale),
        status: parse_status(&row.status)?,
        provider: row
            .provider_key
            .as_deref()
            .map(parse_provider)
            .transpose()?,
        attempt_count: row.attempt_count,
        source_character_count: row.source_character_count,
        error_message: row.error_message,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
    })
}

pub fn normalize_locale(value: &str) -> Result<String> {
    let value = value.trim().replace('_', "-");
    let mut parts = value.split('-');
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    if language.len() != 2 || !language.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(anyhow!("locale must be a BCP 47 language tag"));
    }
    Ok(match parts.next() {
        Some(region) if region.len() == 2 => format!("{language}-{}", region.to_ascii_uppercase()),
        Some(_) => return Err(anyhow!("locale region must contain two letters")),
        None => language,
    })
}

pub fn normalize_locales(values: &[String]) -> Result<Vec<String>> {
    let mut result = values
        .iter()
        .map(|value| normalize_locale(value))
        .collect::<Result<Vec<_>>>()?;
    result.sort();
    result.dedup();
    Ok(result)
}

pub(super) fn provider_response(
    mut provider: StoredTranslationProvider,
    usage: i64,
) -> Result<TranslationProviderResponse> {
    if provider.provider_key == "deepl" && clean(provider.endpoint.as_deref()).is_none() {
        provider.endpoint = Some(deepl_endpoint(provider.deepl_plan.as_deref()).to_string());
    }
    Ok(TranslationProviderResponse {
        provider: parse_provider(&provider.provider_key)?,
        enabled: provider.enabled,
        priority: provider.priority,
        endpoint: provider.endpoint,
        has_api_key: provider
            .api_key
            .as_ref()
            .is_some_and(|value| !value.is_empty()),
        model: provider.model,
        llm_api_kind: provider
            .llm_api_kind
            .as_deref()
            .map(parse_llm_api_kind)
            .transpose()?,
        deepl_plan: provider
            .deepl_plan
            .as_deref()
            .map(parse_deepl_plan)
            .transpose()?,
        monthly_character_limit: provider.monthly_character_limit,
        current_month_characters: usage,
    })
}

pub(super) fn provider_endpoint(provider: &TranslationProviderInput) -> Option<String> {
    clean(provider.endpoint.as_deref()).or_else(|| {
        (provider.provider == TranslationProviderKind::Deepl)
            .then(|| deepl_endpoint(provider.deepl_plan.map(deepl_plan)).to_string())
    })
}

fn deepl_endpoint(plan: Option<&str>) -> &'static str {
    if plan == Some("pro") {
        "https://api.deepl.com"
    } else {
        "https://api-free.deepl.com"
    }
}

pub(super) fn validate_provider_inputs(providers: &[TranslationProviderInput]) -> Result<()> {
    let mut priorities = std::collections::HashSet::new();
    for provider in providers {
        if !priorities.insert(provider.priority) {
            return Err(anyhow!("translation provider priorities must be unique"));
        }
        if provider
            .monthly_character_limit
            .is_some_and(|limit| limit <= 0)
        {
            return Err(anyhow!("monthly character limit must be positive"));
        }
    }
    Ok(())
}

pub(super) fn validate_glossary(values: &[TranslationGlossaryEntry]) -> Result<()> {
    if values.len() > 500
        || values
            .iter()
            .any(|item| item.source.trim().is_empty() || item.target.trim().is_empty())
    {
        return Err(anyhow!("glossary requires 0..=500 non-empty term pairs"));
    }
    Ok(())
}

pub(super) fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn provider_key(value: TranslationProviderKind) -> &'static str {
    match value {
        TranslationProviderKind::Deepl => "deepl",
        TranslationProviderKind::Llm => "llm",
        TranslationProviderKind::Libretranslate => "libretranslate",
    }
}

pub(super) fn llm_api_kind(value: TranslationLlmApiKind) -> &'static str {
    match value {
        TranslationLlmApiKind::OpenaiResponses => "openai_responses",
        TranslationLlmApiKind::OpenaiChatCompletions => "openai_chat_completions",
        TranslationLlmApiKind::AnthropicMessages => "anthropic_messages",
    }
}

pub(super) fn deepl_plan(value: DeeplPlan) -> &'static str {
    match value {
        DeeplPlan::Free => "free",
        DeeplPlan::Pro => "pro",
    }
}

fn parse_provider(value: &str) -> Result<TranslationProviderKind> {
    match value {
        "deepl" => Ok(TranslationProviderKind::Deepl),
        "llm" => Ok(TranslationProviderKind::Llm),
        "libretranslate" => Ok(TranslationProviderKind::Libretranslate),
        _ => Err(anyhow!("invalid translation provider")),
    }
}

fn parse_llm_api_kind(value: &str) -> Result<TranslationLlmApiKind> {
    match value {
        "openai_responses" => Ok(TranslationLlmApiKind::OpenaiResponses),
        "openai_chat_completions" => Ok(TranslationLlmApiKind::OpenaiChatCompletions),
        "anthropic_messages" => Ok(TranslationLlmApiKind::AnthropicMessages),
        _ => Err(anyhow!("invalid translation LLM api kind")),
    }
}

fn parse_deepl_plan(value: &str) -> Result<DeeplPlan> {
    match value {
        "free" => Ok(DeeplPlan::Free),
        "pro" => Ok(DeeplPlan::Pro),
        _ => Err(anyhow!("invalid DeepL plan")),
    }
}

fn parse_status(value: &str) -> Result<TranslationStatus> {
    match value {
        "queued" => Ok(TranslationStatus::Queued),
        "running" => Ok(TranslationStatus::Running),
        "succeeded" => Ok(TranslationStatus::Succeeded),
        "failed" => Ok(TranslationStatus::Failed),
        "skipped" => Ok(TranslationStatus::Skipped),
        "quota_exceeded" => Ok(TranslationStatus::QuotaExceeded),
        _ => Err(anyhow!("invalid translation status")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepl_endpoint_defaults_follow_plan() {
        assert_eq!(deepl_endpoint(Some("free")), "https://api-free.deepl.com");
        assert_eq!(deepl_endpoint(Some("pro")), "https://api.deepl.com");
        assert_eq!(deepl_endpoint(None), "https://api-free.deepl.com");
    }
}
