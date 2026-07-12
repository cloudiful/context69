mod deepl;
mod libretranslate;
mod llm;

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use context69_contracts::TranslationGlossaryEntry;

use crate::{segmenter::TranslationSegment, store::StoredTranslationProvider};

#[derive(Debug, Clone)]
pub struct ProviderTranslationRequest<'a> {
    pub source_locale: Option<&'a str>,
    pub target_locale: &'a str,
    pub segments: &'a [TranslationSegment],
    pub glossary: &'a [TranslationGlossaryEntry],
}

#[derive(Debug, Clone)]
pub struct ProviderTranslationResult {
    pub translations: HashMap<String, String>,
    pub model_name: Option<String>,
}

#[async_trait]
trait TranslationProvider: Send + Sync {
    async fn translate(
        &self,
        request: &ProviderTranslationRequest<'_>,
    ) -> Result<ProviderTranslationResult>;
}

pub async fn translate(
    client: &reqwest::Client,
    provider: &StoredTranslationProvider,
    request: &ProviderTranslationRequest<'_>,
) -> Result<ProviderTranslationResult> {
    let result = match provider.provider_key.as_str() {
        "deepl" => {
            deepl::DeepLProvider::new(client.clone(), provider.clone())
                .translate(request)
                .await
        }
        "llm" => {
            llm::LlmProvider::new(client.clone(), provider.clone())
                .translate(request)
                .await
        }
        "libretranslate" => {
            libretranslate::LibreTranslateProvider::new(client.clone(), provider.clone())
                .translate(request)
                .await
        }
        other => Err(anyhow!("unsupported translation provider {other}")),
    }?;
    validate_result(request, &result)?;
    Ok(result)
}

fn validate_result(
    request: &ProviderTranslationRequest<'_>,
    result: &ProviderTranslationResult,
) -> Result<()> {
    if result.translations.len() != request.segments.len() {
        return Err(anyhow!("provider returned an incomplete segment set"));
    }
    for segment in request.segments {
        if result
            .translations
            .get(&segment.id)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(anyhow!(
                "provider omitted translation segment {}",
                segment.id
            ));
        }
    }
    Ok(())
}

pub fn source_character_count(segments: &[TranslationSegment]) -> i64 {
    segments
        .iter()
        .map(|segment| segment.text.chars().count() as i64)
        .sum()
}

pub fn language_code(locale: &str) -> String {
    locale
        .split('-')
        .next()
        .unwrap_or(locale)
        .to_ascii_uppercase()
}
