use anyhow::Result;

use crate::domain_errors::DomainError;

use crate::{
    contracts::{
        CanonicalUpdateSearchSettingsRequest, UpdateCodeEmbeddingSettings,
        UpdateDoclingSettingsRequest, UpdateRuntimeEmbeddingSettings,
        UpdateRuntimeFileLibrarySettings, UpdateRuntimeSettingsRequest,
        UpdateSearchSettingsRequest,
    },
    db::{StoredRuntimeImageEmbeddingSettings, StoredSearchSettings},
    support::normalize::normalize_optional_string,
};

use super::inheritance::{DocumentEmbeddingSource, resolve_code_settings, resolve_image_settings};
use super::runtime_mappers::{
    image_embedding_config, stored_code_embedding_from_request, stored_image_embedding_from_request,
};

use context69_contracts::settings::DOCLING_OCR_PRESET_MAX_LEN;
use context69_contracts::settings::{ChunkingTokenizer, ChunkingUnit};

pub(super) fn runtime_settings_request(request: &UpdateRuntimeSettingsRequest) -> Result<()> {
    if request.qdrant.url.trim().is_empty() {
        return Err(DomainError::invalid_argument("runtime.qdrant.url must not be empty").into());
    }
    if request.qdrant.collection_name.trim().is_empty() {
        return Err(DomainError::invalid_argument(
            "runtime.qdrant.collection_name must not be empty",
        )
        .into());
    }
    if request.embedding.base_url.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.embedding.base_url must not be empty").into(),
        );
    }
    if request.embedding.model.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.embedding.model must not be empty").into(),
        );
    }
    if request.embedding.dimensions == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.embedding.dimensions must be greater than 0",
        )
        .into());
    }
    if request.embedding.timeout_secs == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.embedding.timeout_secs must be greater than 0",
        )
        .into());
    }
    if let Some(code) = &request.code_embedding {
        code_embedding_request(code, &request.embedding)?;
    }
    if let Some(image) = &request.image_embedding {
        image_embedding_request(image, &request.embedding, &request.qdrant.collection_name)?;
    }
    if request.scheduler.interval_secs == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.scheduler.interval_secs must be greater than 0",
        )
        .into());
    }
    if request.scheduler.max_concurrency == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.scheduler.max_concurrency must be greater than 0",
        )
        .into());
    }
    if request.scheduler.job_id.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.scheduler.job_id must not be empty").into(),
        );
    }
    if request.chunking.max_chars == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.chunking.max_chars must be greater than 0",
        )
        .into());
    }
    if request.chunking.overlap_chars >= request.chunking.max_chars {
        return Err(DomainError::invalid_argument(
            "runtime.chunking.overlap_chars must be smaller than runtime.chunking.max_chars",
        )
        .into());
    }
    // The token integers are persisted as PostgreSQL BIGINT in both modes, so a
    // value the column cannot hold must be refused here, before any settings or
    // secret write, instead of failing the settings transaction after the
    // secret patches were already committed. The active-mode rules below still
    // apply on top of this storage check.
    if i64::try_from(request.chunking.max_tokens).is_err() {
        return Err(DomainError::invalid_argument(format!(
            "runtime.chunking.max_tokens must be at most {}",
            i64::MAX
        ))
        .into());
    }
    if i64::try_from(request.chunking.overlap_tokens).is_err() {
        return Err(DomainError::invalid_argument(format!(
            "runtime.chunking.overlap_tokens must be at most {}",
            i64::MAX
        ))
        .into());
    }
    match request.chunking.tokenizer {
        ChunkingTokenizer::HfLocal => {
            let sha256 = request
                .chunking
                .tokenizer_sha256
                .as_deref()
                .ok_or_else(|| {
                    DomainError::invalid_argument(
                        "runtime.chunking.tokenizer_sha256 is required for hf_local",
                    )
                })?;
            if !crate::chunking::chunking_token_artifact::is_lower_sha256(sha256) {
                return Err(DomainError::invalid_argument(
                    "runtime.chunking.tokenizer_sha256 must be a lowercase 64-hex artifact hash",
                )
                .into());
            }
        }
        _ => {
            if request.chunking.tokenizer_sha256.is_some() {
                return Err(DomainError::invalid_argument(
                    "runtime.chunking.tokenizer_sha256 must be absent for a built-in tokenizer",
                )
                .into());
            }
        }
    }
    if request.chunking.unit == ChunkingUnit::Tokens {
        if request.chunking.max_tokens == 0 {
            return Err(DomainError::invalid_argument(
                "runtime.chunking.max_tokens must be greater than 0",
            )
            .into());
        }
        if request.chunking.overlap_tokens >= request.chunking.max_tokens {
            return Err(DomainError::invalid_argument(
                "runtime.chunking.overlap_tokens must be smaller than runtime.chunking.max_tokens",
            )
            .into());
        }
    }
    if request.file_library.storage_root.trim().is_empty() {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.storage_root must not be empty",
        )
        .into());
    }
    if request.file_library.max_upload_size_mb == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.max_upload_size_mb must be greater than 0",
        )
        .into());
    }
    if let Some(s3) = &request.file_library.s3 {
        for (name, value) in [
            ("endpoint", s3.endpoint.as_str()),
            ("region", s3.region.as_str()),
            ("bucket", s3.bucket.as_str()),
            ("access_key", s3.access_key.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(DomainError::invalid_argument(format!(
                    "runtime.file_library.s3.{name} must not be empty"
                ))
                .into());
            }
        }
    }
    registry_capacity_request(&request.file_library)?;
    Ok(())
}

/// The code block is optional and inherits the document block field by field.
///
/// Every field is an override: an absent or blank one inherits the document
/// value, so an enabled block needs no provider fields of its own. An enabled
/// block must still resolve to a usable configuration: either it overrides
/// what it needs, or the document block provides it.
///
/// An explicitly supplied number is a value, never an inheritance: a zero is
/// refused here, before the sparse projection normalizes anything. Every
/// present number must also be representable in PostgreSQL.
fn code_embedding_request(
    request: &UpdateCodeEmbeddingSettings,
    document: &UpdateRuntimeEmbeddingSettings,
) -> Result<()> {
    if let Some(dimensions) = request.dimensions {
        if dimensions == 0 {
            return Err(DomainError::invalid_argument(
                "runtime.code_embedding.dimensions must be greater than 0",
            )
            .into());
        }
        if i64::try_from(dimensions).is_err() {
            return Err(DomainError::invalid_argument(format!(
                "runtime.code_embedding.dimensions must be at most {}",
                i64::MAX
            ))
            .into());
        }
    }
    if let Some(timeout_secs) = request.timeout_secs {
        if timeout_secs == 0 {
            return Err(DomainError::invalid_argument(
                "runtime.code_embedding.timeout_secs must be greater than 0",
            )
            .into());
        }
        if i64::try_from(timeout_secs).is_err() {
            return Err(DomainError::invalid_argument(format!(
                "runtime.code_embedding.timeout_secs must be at most {}",
                i64::MAX
            ))
            .into());
        }
    }
    if request.enabled {
        let source = DocumentEmbeddingSource::from_update(document, None);
        let overrides = stored_code_embedding_from_request(request);
        if resolve_code_settings(&overrides, Some(&source)).is_none() {
            return Err(DomainError::invalid_argument(
                "runtime.code_embedding must provide a base_url, a model, dimensions, and a \
                 timeout_secs, or inherit them from the document embedding configuration",
            )
            .into());
        }
    }
    Ok(())
}

/// The image block is optional and inherits the document block field by field.
///
/// The collection name is no exception: an absent one resolves to the default
/// image collection, which is never the document collection and never a code
/// collection.
fn image_embedding_request(
    request: &crate::contracts::settings::UpdateImageEmbeddingSettings,
    document: &UpdateRuntimeEmbeddingSettings,
    document_collection: &str,
) -> Result<()> {
    if let Some(dimensions) = request.dimensions {
        if dimensions == 0 {
            return Err(DomainError::invalid_argument(
                "runtime.image_embedding.dimensions must be greater than 0",
            )
            .into());
        }
        if i64::try_from(dimensions).is_err() {
            return Err(DomainError::invalid_argument(format!(
                "runtime.image_embedding.dimensions must be at most {}",
                i64::MAX
            ))
            .into());
        }
    }
    if let Some(timeout_secs) = request.timeout_secs {
        if timeout_secs == 0 {
            return Err(DomainError::invalid_argument(
                "runtime.image_embedding.timeout_secs must be greater than 0",
            )
            .into());
        }
        if i64::try_from(timeout_secs).is_err() {
            return Err(DomainError::invalid_argument(format!(
                "runtime.image_embedding.timeout_secs must be at most {}",
                i64::MAX
            ))
            .into());
        }
    }
    if request.enabled {
        let source = DocumentEmbeddingSource::from_update(document, None);
        let overrides = stored_image_embedding_from_request(request);
        let Some(effective) = resolve_image_settings(&overrides, Some(&source)) else {
            return Err(DomainError::invalid_argument(
                "runtime.image_embedding must provide a base_url, a model, dimensions, and a \
                 timeout_secs, or inherit them from the document embedding configuration",
            )
            .into());
        };
        if effective.collection_name.trim() == document_collection.trim() {
            return Err(DomainError::invalid_argument(
                "runtime.image_embedding.collection_name must not reuse the document collection",
            )
            .into());
        }
        if effective
            .collection_name
            .trim()
            .starts_with(crate::qdrant_index::code_vectors::CODE_COLLECTION_PREFIX)
        {
            return Err(DomainError::invalid_argument(
                "runtime.image_embedding.collection_name must not reuse a code collection",
            )
            .into());
        }
    }
    Ok(())
}

/// The persisted channels still own their collections when a request omits them.
///
/// A document or Qdrant save that omits the image block writes no image row,
/// but an enabled image channel keeps writing its own collection. The candidate
/// document target is therefore compared with the effective stored image
/// destination as well, so a document collection can never move onto the
/// collection an enabled image channel already owns. A request that carries the
/// image block is validated with the block itself, and a disabled channel owns
/// no destination.
pub(super) fn stored_channel_collections(
    request: &UpdateRuntimeSettingsRequest,
    stored_image: Option<&StoredRuntimeImageEmbeddingSettings>,
) -> Result<()> {
    if request.image_embedding.is_some() {
        return Ok(());
    }
    let Some(image) = stored_image.filter(|image| image.enabled) else {
        return Ok(());
    };
    // The save resolves the omitted block against the document candidate of the
    // same request, so the comparison names the destination this save installs.
    let source = DocumentEmbeddingSource::from_update(&request.embedding, None);
    let Some(effective) = resolve_image_settings(image, Some(&source)) else {
        return Ok(());
    };
    let document_collection = request.qdrant.collection_name.trim();
    let namespace = effective.collection_name.trim();
    if namespace == document_collection {
        return Err(DomainError::invalid_argument(
            "runtime.qdrant.collection_name must not reuse the image collection namespace",
        )
        .into());
    }
    let config = image_embedding_config(&effective, None);
    let collection = crate::image_embedding_vectors::image_collection_name(
        namespace,
        &crate::image_embedding::image_embedding_fingerprint(&config),
    );
    if collection == document_collection {
        return Err(DomainError::invalid_argument(
            "runtime.qdrant.collection_name must not alias the image collection",
        )
        .into());
    }
    Ok(())
}

/// The registry capacity caps a request carries.
///
/// A cap the request omits keeps the stored one, so only the caps it carries
/// are compared here. The save compares the effective policy against the
/// stored one, so a request that mixes a new cap with a stored cap is still
/// refused before anything is written.
fn registry_capacity_request(file_library: &UpdateRuntimeFileLibrarySettings) -> Result<()> {
    let caps = [
        (
            "registry_max_ids_per_file",
            file_library.registry_max_ids_per_file,
        ),
        (
            "registry_max_ids_per_run",
            file_library.registry_max_ids_per_run,
        ),
    ];
    for (name, value) in caps {
        if let Some(value) = value
            && value <= 0
        {
            return Err(DomainError::invalid_argument(format!(
                "runtime.file_library.{name} must be greater than 0"
            ))
            .into());
        }
    }
    if let (Some(run), Some(file)) = (
        file_library.registry_max_ids_per_run,
        file_library.registry_max_ids_per_file,
    ) && run > file
    {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.registry_max_ids_per_run must not exceed \
             runtime.file_library.registry_max_ids_per_file",
        )
        .into());
    }
    Ok(())
}

/// Validates a Docling save against the effective OCR toggle.
///
/// A disabled OCR section hides the preset, so a blank submitted preset is an
/// untouched hidden field rather than a save blocker; it is resolved back to
/// the stored selection or the default. An enabled section keeps the strict
/// preset rules, and a nonblank submitted preset is always validated because it
/// is what the save persists.
pub(super) fn docling_request(
    request: &UpdateDoclingSettingsRequest,
    do_ocr: bool,
    do_picture_description: bool,
) -> Result<()> {
    let base_url = request.connection.base_url.trim();
    if base_url.is_empty() {
        return Err(DomainError::invalid_argument("docling.base_url must not be empty").into());
    }
    if request.connection.timeout_secs == 0 {
        return Err(
            DomainError::invalid_argument("docling.timeout_secs must be greater than 0").into(),
        );
    }
    if request.connection.poll_interval_secs == 0 {
        return Err(DomainError::invalid_argument(
            "docling.poll_interval_secs must be greater than 0",
        )
        .into());
    }
    if request.connection.task_timeout_secs == 0 {
        return Err(DomainError::invalid_argument(
            "docling.task_timeout_secs must be greater than 0",
        )
        .into());
    }
    if request.connection.max_inflight < context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN
        || request.connection.max_inflight > context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX
    {
        return Err(DomainError::invalid_argument(format!(
            "docling.max_inflight must be between {} and {}",
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX
        ))
        .into());
    }
    match &request.ocr_preset {
        // An old client that does not know the setting keeps the stored one.
        None => {}
        Some(None) => {
            return Err(
                DomainError::invalid_argument("docling.ocr_preset must not be null").into(),
            );
        }
        Some(Some(preset)) => {
            let preset = preset.trim();
            if preset.is_empty() {
                if do_ocr {
                    return Err(DomainError::invalid_argument(
                        "docling.ocr_preset must not be empty",
                    )
                    .into());
                }
            } else if preset.len() > DOCLING_OCR_PRESET_MAX_LEN {
                return Err(DomainError::invalid_argument(format!(
                    "docling.ocr_preset must be at most {DOCLING_OCR_PRESET_MAX_LEN} characters"
                ))
                .into());
            } else if !preset.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            }) {
                return Err(DomainError::invalid_argument(
                    "docling.ocr_preset must contain only ASCII letters, digits, underscore, hyphen, or dot",
                )
                .into());
            }
        }
    }
    if do_picture_description {
        docling_vlm_mode_selection(request)?;
    }

    Ok(())
}

/// Rejects an explicit VLM mode that contradicts the fields it submits.
///
/// A disabled section skips this check because its hidden branch cannot block
/// a save; its supplied key is still staged by the service instead of being
/// discarded.
fn docling_vlm_mode_selection(request: &UpdateDoclingSettingsRequest) -> Result<()> {
    use context69_contracts::settings::DoclingVlmMode;

    let Some(mode) = request.vlm_mode else {
        return Ok(());
    };
    let vlm = request.vlm.as_ref();
    let preset =
        vlm.and_then(|vlm| normalize_optional_string(vlm.picture_description_preset.clone()));
    let custom = [
        vlm.and_then(|vlm| normalize_optional_string(vlm.openai_base_url.clone())),
        vlm.and_then(|vlm| normalize_optional_string(vlm.api_key.clone())),
        vlm.and_then(|vlm| normalize_optional_string(vlm.vlm_pipeline_model.clone())),
        vlm.and_then(|vlm| normalize_optional_string(vlm.picture_description_model.clone())),
        vlm.and_then(|vlm| normalize_optional_string(vlm.code_formula_model.clone())),
    ]
    .into_iter()
    .any(|value| value.is_some());

    let inconsistent = match mode {
        DoclingVlmMode::Custom => preset.is_some(),
        DoclingVlmMode::Preset => custom,
        DoclingVlmMode::Disabled => preset.is_some() || custom,
    };
    if inconsistent {
        return Err(DomainError::invalid_argument(
            "docling.vlm_mode must match the submitted VLM branch fields",
        )
        .into());
    }
    Ok(())
}

pub(super) fn search_request(request: &UpdateSearchSettingsRequest) -> Result<()> {
    let canonical = CanonicalUpdateSearchSettingsRequest::from(request.clone());
    canonical_search_request(&canonical)
}

pub(super) fn canonical_search_request(
    request: &CanonicalUpdateSearchSettingsRequest,
) -> Result<()> {
    if request.rerank_base_url.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("search.rerank_base_url must not be empty").into(),
        );
    }
    if request.rerank_model.trim().is_empty() {
        return Err(DomainError::invalid_argument("search.rerank_model must not be empty").into());
    }
    if request.candidate_limit == 0 {
        return Err(
            DomainError::invalid_argument("search.candidate_limit must be greater than 0").into(),
        );
    }
    if request.timeout_secs == 0 {
        return Err(
            DomainError::invalid_argument("search.timeout_secs must be greater than 0").into(),
        );
    }
    validate_fusion_weights(request.vector_weight, request.keyword_weight)?;
    Ok(())
}

pub(super) fn stored_search_settings(settings: &StoredSearchSettings) -> Result<()> {
    if settings.rerank_base_url.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("search.rerank_base_url must not be empty").into(),
        );
    }
    if settings.rerank_model.trim().is_empty() {
        return Err(DomainError::invalid_argument("search.rerank_model must not be empty").into());
    }
    if settings.candidate_limit == 0 {
        return Err(
            DomainError::invalid_argument("search.candidate_limit must be greater than 0").into(),
        );
    }
    if settings.timeout_secs == 0 {
        return Err(
            DomainError::invalid_argument("search.timeout_secs must be greater than 0").into(),
        );
    }
    validate_fusion_weights(settings.vector_weight, settings.keyword_weight)?;
    Ok(())
}

/// Hybrid fusion weights must stay in [0, 1] and may not exceed the unit
/// budget together: the boost weight is the residual margin
/// (1 - vector - keyword), so it cannot be negative.
fn validate_fusion_weights(vector_weight: f32, keyword_weight: f32) -> Result<()> {
    if !(0.0..=1.0).contains(&vector_weight) {
        return Err(
            DomainError::invalid_argument("search.vector_weight must be between 0 and 1").into(),
        );
    }
    if !(0.0..=1.0).contains(&keyword_weight) {
        return Err(
            DomainError::invalid_argument("search.keyword_weight must be between 0 and 1").into(),
        );
    }
    if vector_weight + keyword_weight > 1.0 {
        return Err(DomainError::invalid_argument(
            "search.vector_weight and search.keyword_weight must not sum above 1",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod validation_tests {
    use super::{
        code_embedding_request, image_embedding_request, registry_capacity_request,
        stored_channel_collections,
    };
    use crate::contracts::UpdateRuntimeEmbeddingSettings;
    use crate::contracts::UpdateRuntimeFileLibrarySettings;
    use crate::contracts::UpdateRuntimeSettingsRequest;
    use crate::contracts::settings::UpdateImageEmbeddingSettings;
    use crate::db::StoredRuntimeImageEmbeddingSettings;
    use crate::services::settings::inheritance::{DocumentEmbeddingSource, resolve_image_settings};
    use crate::services::settings::runtime_mappers::image_embedding_config;
    use context69_contracts::settings::SecretPatch;
    use context69_contracts::settings::UpdateCodeEmbeddingSettings;
    use context69_contracts::settings::{ChunkingTokenizer, ChunkingUnit};

    fn file_library(
        per_file: Option<i64>,
        per_run: Option<i64>,
    ) -> UpdateRuntimeFileLibrarySettings {
        UpdateRuntimeFileLibrarySettings {
            storage_root: "/tmp/library".to_string(),
            max_upload_size_mb: 16,
            trusted_proxy_enabled: false,
            registry_max_ids_per_file: per_file,
            registry_max_ids_per_run: per_run,
            s3: None,
        }
    }

    fn image(enabled: bool) -> UpdateImageEmbeddingSettings {
        UpdateImageEmbeddingSettings {
            enabled,
            base_url: if enabled {
                Some("https://images.invalid/v1".to_string())
            } else {
                None
            },
            model: if enabled {
                Some("image-model".to_string())
            } else {
                None
            },
            query_model: None,
            dimensions: if enabled { Some(768) } else { None },
            timeout_secs: if enabled { Some(30) } else { None },
            collection_name: if enabled {
                Some("context69_images".to_string())
            } else {
                None
            },
            api_key: SecretPatch::Keep,
        }
    }

    fn code(enabled: bool) -> UpdateCodeEmbeddingSettings {
        UpdateCodeEmbeddingSettings {
            enabled,
            base_url: if enabled {
                Some("https://embedding.invalid/v1".to_string())
            } else {
                None
            },
            model: if enabled {
                Some("code-model".to_string())
            } else {
                None
            },
            query_model: None,
            dimensions: if enabled { Some(1024) } else { None },
            timeout_secs: if enabled { Some(30) } else { None },
            api_key: SecretPatch::Keep,
        }
    }

    fn document() -> UpdateRuntimeEmbeddingSettings {
        UpdateRuntimeEmbeddingSettings {
            base_url: "https://documents.invalid/v1".to_string(),
            model: "document-model".to_string(),
            query_model: None,
            dimensions: 768,
            timeout_secs: 30,
            api_key: None,
            confirm_vector_space_unchanged: false,
        }
    }

    #[test]
    fn a_disabled_block_needs_no_provider_fields() {
        code_embedding_request(&code(false), &document())
            .expect("a disabled block may keep blank fields");
        image_embedding_request(&image(false), &document(), "context69")
            .expect("a disabled image block may keep blanks");
    }

    #[test]
    fn an_enabled_block_inherits_the_document_configuration() {
        let mut request = code(true);
        request.base_url = None;
        request.model = None;
        request.dimensions = None;
        request.timeout_secs = None;
        code_embedding_request(&request, &document())
            .expect("an enabled block may inherit every field");

        let mut request = image(true);
        request.base_url = None;
        request.model = None;
        request.dimensions = None;
        request.timeout_secs = None;
        request.collection_name = None;
        image_embedding_request(&request, &document(), "context69")
            .expect("an enabled image block may inherit every field");
    }

    #[test]
    fn an_enabled_block_without_any_usable_configuration_is_refused() {
        let mut document = document();
        document.base_url = String::new();
        document.model = String::new();
        let mut request = code(true);
        request.base_url = None;
        request.model = None;
        request.dimensions = None;
        request.timeout_secs = None;
        assert!(code_embedding_request(&request, &document).is_err());
    }

    #[test]
    fn an_enabled_image_block_cannot_alias_a_document_or_code_collection() {
        let mut request = image(true);
        request.collection_name = Some("context69".to_string());
        assert!(image_embedding_request(&request, &document(), "context69").is_err());
        let mut request = image(true);
        request.collection_name = Some(crate::qdrant_index::code_vectors::code_collection_name(
            "fingerprint",
        ));
        assert!(image_embedding_request(&request, &document(), "context69").is_err());
    }

    #[test]
    fn an_inherited_image_collection_resolves_to_the_default_namespace() {
        let mut request = image(true);
        request.collection_name = None;
        image_embedding_request(&request, &document(), "context69")
            .expect("the default image collection never aliases the document collection");
    }

    #[test]
    fn an_explicit_zero_is_refused_and_never_read_as_an_inheritance() {
        let mut request = code(true);
        request.dimensions = Some(0);
        assert!(
            code_embedding_request(&request, &document()).is_err(),
            "an explicit zero is a value, not an inheritance"
        );
        let mut request = code(true);
        request.timeout_secs = Some(0);
        assert!(code_embedding_request(&request, &document()).is_err());
        let mut request = image(true);
        request.dimensions = Some(0);
        assert!(image_embedding_request(&request, &document(), "context69").is_err());
        let mut request = image(true);
        request.timeout_secs = Some(0);
        assert!(image_embedding_request(&request, &document(), "context69").is_err());
        // An absent number is the inheritance, and it stays valid.
        let mut request = code(true);
        request.dimensions = None;
        request.timeout_secs = None;
        code_embedding_request(&request, &document())
            .expect("an absent number inherits the document value");
    }

    #[test]
    fn a_blank_override_inherits_instead_of_failing() {
        let mut request = code(true);
        request.base_url = Some(String::new());
        request.model = Some(String::new());
        code_embedding_request(&request, &document())
            .expect("a blank override inherits the document value");
    }

    #[test]
    fn omitted_registry_caps_keep_the_stored_policy() {
        registry_capacity_request(&file_library(None, None))
            .expect("an omitted cap is not a validation error");
        registry_capacity_request(&file_library(Some(8), None))
            .expect("a partial policy is resolved against the stored one");
        registry_capacity_request(&file_library(None, Some(8)))
            .expect("a partial policy is resolved against the stored one");
    }

    #[test]
    fn registry_caps_must_be_positive_and_ordered() {
        registry_capacity_request(&file_library(Some(8), Some(8)))
            .expect("the run cap may equal the file cap");
        assert!(registry_capacity_request(&file_library(Some(0), Some(8))).is_err());
        assert!(registry_capacity_request(&file_library(Some(8), Some(0))).is_err());
        assert!(
            registry_capacity_request(&file_library(Some(8), Some(9))).is_err(),
            "the run cap may not exceed the file cap"
        );
    }

    fn runtime_request(
        collection: &str,
        image_block: Option<UpdateImageEmbeddingSettings>,
    ) -> UpdateRuntimeSettingsRequest {
        UpdateRuntimeSettingsRequest {
            qdrant: crate::contracts::RuntimeQdrantSettings {
                url: "http://qdrant.invalid:6334".to_string(),
                collection_name: collection.to_string(),
                recreate_on_dimension_mismatch: false,
            },
            embedding: document(),
            code_embedding: None,
            image_embedding: image_block,
            scheduler: crate::contracts::RuntimeSchedulerSettings {
                interval_secs: 300,
                run_on_start: true,
                max_concurrency: 1,
                job_id: "context69-sync".to_string(),
                valkey_url: None,
            },
            chunking: crate::contracts::RuntimeChunkingSettings {
                max_chars: 1200,
                overlap_chars: 200,
                unit: ChunkingUnit::Chars,
                tokenizer: ChunkingTokenizer::Cl100kBase,
                tokenizer_sha256: None,
                max_tokens: 1024,
                overlap_tokens: 128,
            },
            file_library: file_library(None, None),
        }
    }

    fn stored_image(enabled: bool, namespace: Option<&str>) -> StoredRuntimeImageEmbeddingSettings {
        StoredRuntimeImageEmbeddingSettings {
            enabled,
            base_url: None,
            collection_name: namespace.map(str::to_string),
            model: None,
            query_model: None,
            dimensions: None,
            timeout_secs: None,
            api_key: None,
        }
    }

    #[test]
    fn a_document_save_may_not_alias_the_stored_image_namespace() {
        let request = runtime_request("context69_images", None);
        let error = stored_channel_collections(&request, Some(&stored_image(true, None)))
            .expect_err("the document collection may not reuse the image namespace");
        assert!(error.to_string().contains("image collection"), "{error}");
    }

    #[test]
    fn a_document_save_may_not_alias_the_derived_image_collection() {
        let stored = stored_image(true, Some("image-namespace"));
        let effective = resolve_image_settings(
            &stored,
            Some(&DocumentEmbeddingSource::from_update(&document(), None)),
        )
        .expect("the stored image channel resolves against the document candidate");
        let physical = crate::image_embedding_vectors::image_collection_name(
            "image-namespace",
            &crate::image_embedding::image_embedding_fingerprint(&image_embedding_config(
                &effective, None,
            )),
        );
        let request = runtime_request(&physical, None);
        let error = stored_channel_collections(&request, Some(&stored))
            .expect_err("the document collection may not alias the physical image collection");
        assert!(error.to_string().contains("image collection"), "{error}");
    }

    #[test]
    fn an_omitted_disabled_image_block_owns_no_collection() {
        let request = runtime_request("context69_images", None);
        stored_channel_collections(&request, Some(&stored_image(false, None)))
            .expect("a disabled channel owns no destination");
        stored_channel_collections(&request, None)
            .expect("without a stored channel no document collection is blocked");
    }

    #[test]
    fn a_request_that_carries_the_image_block_is_validated_with_the_block() {
        let request = runtime_request("context69", Some(image(true)));
        stored_channel_collections(&request, Some(&stored_image(true, Some("context69"))))
            .expect("the request's own block replaces the stored one");
    }
}
