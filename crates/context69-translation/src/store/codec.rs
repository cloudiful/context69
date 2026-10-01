use anyhow::Result;
use context69_contracts_core::errors::DomainError;
use context69_contracts_translation::{
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
        return Err(DomainError::invalid_argument("locale must be a BCP 47 language tag").into());
    }
    Ok(match parts.next() {
        Some(region) if region.len() == 2 => format!("{language}-{}", region.to_ascii_uppercase()),
        Some(_) => {
            return Err(
                DomainError::invalid_argument("locale region must contain two letters").into(),
            );
        }
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

/// `has_api_key` is supplied by the caller rather than derived from the row:
/// the shared `llm` key may live only in the encrypted store during the
/// transition, and presence has to be answerable without opening it.
pub(super) fn provider_response(
    mut provider: StoredTranslationProvider,
    usage: i64,
    has_api_key: bool,
) -> Result<TranslationProviderResponse> {
    if provider.provider_key == "deepl" && clean(provider.endpoint.as_deref()).is_none() {
        provider.endpoint = Some(deepl_endpoint(provider.deepl_plan.as_deref()).to_string());
    }
    Ok(TranslationProviderResponse {
        provider: parse_provider(&provider.provider_key)?,
        enabled: provider.enabled,
        priority: provider.priority,
        endpoint: provider.endpoint,
        has_api_key,
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
            return Err(DomainError::invalid_argument(
                "translation provider priorities must be unique",
            )
            .into());
        }
        if provider
            .monthly_character_limit
            .is_some_and(|limit| limit <= 0)
        {
            return Err(
                DomainError::invalid_argument("monthly character limit must be positive").into(),
            );
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
        return Err(DomainError::invalid_argument(
            "glossary requires 0..=500 non-empty term pairs",
        )
        .into());
    }
    Ok(())
}

pub(super) fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// The pre-phase presence rule for a provider whose API key stays in the legacy
/// column: any present, non-empty value counts, and whitespace is deliberately
/// not trimmed. Only the shared `llm` row uses the store-aware presence path.
pub(super) fn legacy_has_api_key(api_key: Option<&str>) -> bool {
    api_key.is_some_and(|value| !value.is_empty())
}

/// The pre-phase validation rule for a non-`llm` provider: a trimmed,
/// non-empty legacy key configures it, so a whitespace-only value does not
/// satisfy the enabled-provider check. This differs on purpose from the
/// untrimmed [`legacy_has_api_key`] response projection.
pub(super) fn validation_has_legacy_api_key(api_key: Option<&str>) -> bool {
    clean(api_key).is_some()
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
        _ => Err(DomainError::invalid_argument("invalid translation provider").into()),
    }
}

fn parse_llm_api_kind(value: &str) -> Result<TranslationLlmApiKind> {
    match value {
        "openai_responses" => Ok(TranslationLlmApiKind::OpenaiResponses),
        "openai_chat_completions" => Ok(TranslationLlmApiKind::OpenaiChatCompletions),
        "anthropic_messages" => Ok(TranslationLlmApiKind::AnthropicMessages),
        _ => Err(DomainError::invalid_argument("invalid translation LLM api kind").into()),
    }
}

fn parse_deepl_plan(value: &str) -> Result<DeeplPlan> {
    match value {
        "free" => Ok(DeeplPlan::Free),
        "pro" => Ok(DeeplPlan::Pro),
        _ => Err(DomainError::invalid_argument("invalid DeepL plan").into()),
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
        _ => Err(DomainError::invalid_argument("invalid translation status").into()),
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

    #[test]
    fn provider_config_hash_excludes_the_api_key() {
        let base = crate::store::StoredTranslationProvider {
            provider_key: "llm".to_string(),
            enabled: true,
            priority: 1,
            endpoint: None,
            api_key: None,
            model: Some("gpt".to_string()),
            llm_api_kind: None,
            deepl_plan: None,
            monthly_character_limit: None,
        };
        let mut with_key = base.clone();
        with_key.api_key = Some("placeholder-key".to_string());
        assert_eq!(base.config_hash(), with_key.config_hash());
    }

    #[test]
    fn blank_api_key_maps_to_keep() {
        assert_eq!(clean(None), None);
        assert_eq!(clean(Some("   ")), None);
        assert_eq!(clean(Some("  key  ")), Some("key".to_string()));
    }

    #[test]
    fn non_llm_presence_keeps_the_pre_phase_non_empty_rule() {
        assert!(!legacy_has_api_key(None));
        assert!(!legacy_has_api_key(Some("")));
        // Whitespace-only is a present value for the legacy providers: the
        // projection stays `!value.is_empty()` and is not trimmed.
        assert!(legacy_has_api_key(Some("   ")));
        assert!(legacy_has_api_key(Some("key")));
    }

    #[test]
    fn non_llm_validation_rejects_a_whitespace_only_legacy_key() {
        assert!(!validation_has_legacy_api_key(None));
        assert!(!validation_has_legacy_api_key(Some("")));
        // The enabled-provider check uses the pre-phase trimmed rule.
        assert!(!validation_has_legacy_api_key(Some("   ")));
        assert!(validation_has_legacy_api_key(Some("key")));
    }
}
