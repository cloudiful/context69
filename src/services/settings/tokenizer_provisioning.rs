//! The preparation of one known tokenizer preset.
//!
//! The preparation is an explicit, bounded, administrator-initiated action:
//! it downloads the one pinned `tokenizer.json` of a known preset, verifies
//! its exact size and hash, publishes it into the writable cache directory,
//! and activates it in this process. It never downloads weights, never
//! fetches a model configuration, and never resolves a URL a client chose.
//!
//! The work is serialized per preset, so two requests for the same preset
//! prepare it once, and the wait for that preparation is itself bounded: a
//! request that cannot start in time is refused with an actionable message
//! instead of queueing forever.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use tokio::time::timeout;

use context69_contracts::settings::{
    TokenizerPresetDescriptor, TokenizerProvisionRequest, TokenizerProvisionResponse,
    TokenizerProvisionStatus,
};

use crate::{
    chunking::chunking_token_artifact::{
        PresetSlot, PresetSlots, TokenizerCatalog, parse_verified_tokenizer, verified_preset_engine,
    },
    domain_errors::DomainError,
    tokenizer_artifact_download::{
        TokenizerArtifactDownloader, publish_cached_artifact, sync_published_entry,
    },
    tokenizer_presets::{PRESETS, TokenizerPreset, preset_by_id},
};

/// The time a request may wait for another preparation of the same preset.
pub const TOKENIZER_PROVISION_WAIT: Duration = Duration::from_secs(30);

/// The preparation of the known tokenizer presets.
#[derive(Clone)]
pub struct TokenizerProvisioning {
    cache_dir: PathBuf,
    slots: Arc<PresetSlots>,
    downloader: TokenizerArtifactDownloader,
    wait: Duration,
}

impl std::fmt::Debug for TokenizerProvisioning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenizerProvisioning")
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

impl TokenizerProvisioning {
    /// The preparation of the presets of this process.
    ///
    /// The cache directory and the cells are the ones of the installed
    /// catalog, so a prepared preset is usable by the runtime that already
    /// resolved against it.
    pub fn for_process() -> Result<Self> {
        let catalog = TokenizerCatalog::installed_or_empty();
        let cache_dir = catalog
            .preset_cache_dir()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the tokenizer preset cache directory is not configured for this process"
                )
            })?
            .to_path_buf();
        Ok(Self {
            cache_dir,
            slots: Arc::clone(catalog.preset_slots()),
            downloader: TokenizerArtifactDownloader::new()?,
            wait: TOKENIZER_PROVISION_WAIT,
        })
    }

    /// The preparation of the presets into its own cells.
    ///
    /// A test uses this to keep its activations out of the process-wide cells.
    pub fn with_slots(cache_dir: PathBuf, slots: Arc<PresetSlots>) -> Result<Self> {
        Ok(Self {
            cache_dir,
            slots,
            downloader: TokenizerArtifactDownloader::new()?,
            wait: TOKENIZER_PROVISION_WAIT,
        })
    }

    /// The preparation that retrieves from a fixed URL.
    ///
    /// The loopback tests use it to serve the pinned bytes without reaching
    /// the network. It is deliberately not part of the provisioning API.
    #[cfg(test)]
    pub(crate) fn with_downloader(
        cache_dir: PathBuf,
        slots: Arc<PresetSlots>,
        downloader: TokenizerArtifactDownloader,
    ) -> Self {
        Self {
            cache_dir,
            slots,
            downloader,
            wait: TOKENIZER_PROVISION_WAIT,
        }
    }

    /// The same preparation with a shorter wait for a test.
    #[cfg(test)]
    pub(crate) fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Prepares one known preset.
    ///
    /// An unknown identifier is refused before any HTTP request or filesystem
    /// write. A preset that is already active is reported as such without
    /// any work, and a cached, verified file is activated without a download.
    pub async fn provision(
        &self,
        request: &TokenizerProvisionRequest,
    ) -> Result<TokenizerProvisionResponse> {
        let preset = preset_by_id(request.preset_id.trim())
            .ok_or_else(|| unknown_preset(request.preset_id.trim()))?;
        let slot = self
            .slots
            .slot(preset.id)
            .ok_or_else(|| unknown_preset(preset.id))?;

        if slot.engine().is_some() {
            return Ok(response(
                preset,
                TokenizerProvisionStatus::AlreadyReady,
                true,
            ));
        }

        let status = self.prepare(preset, slot).await?;
        Ok(response(preset, status, true))
    }

    /// Prepares one preset into its cell.
    ///
    /// The work is serialized by the cell, so concurrent requests prepare the
    /// preset once and later ones observe it as already ready.
    async fn prepare(
        &self,
        preset: &'static TokenizerPreset,
        slot: &PresetSlot,
    ) -> Result<TokenizerProvisionStatus> {
        // The wait for the preparation of this preset is bounded, so a request
        // that cannot start in time is refused instead of queueing.
        let _guard = timeout(self.wait, slot.lock().lock()).await.map_err(|_| {
            DomainError::conflict(
                "another preparation of this tokenizer preset is still running; retry shortly",
            )
        })?;

        // Another request may have prepared the preset while this one waited.
        if slot.engine().is_some() {
            return Ok(TokenizerProvisionStatus::AlreadyReady);
        }

        let cached = self.cache_dir.join(format!("{}.json", preset.sha256));
        // An entry that already occupies the published name is never replaced:
        // it is activated only when it verifies, and an unusable entry stays
        // exactly as the operator left it. A dangling symlink is an entry too,
        // so it is refused instead of being silently replaced by a download.
        match std::fs::symlink_metadata(&cached) {
            Ok(_) => {
                // Verify and parse without filling the cell: the cell is only
                // filled once the file and its directory entry are durable, so
                // a failed flush never makes a retry look already ready.
                let engine = match verified_preset_engine(&cached, preset) {
                    Ok(engine) => engine,
                    Err(error) => {
                        tracing::warn!(
                            preset = preset.id,
                            error = %format!("{error:#}"),
                            "the cached tokenizer preset is not usable; it is left untouched"
                        );
                        return Err(DomainError::conflict(
                            "the cached tokenizer file for this preset is not usable; remove it and prepare the preset again",
                        )
                        .into());
                    }
                };
                // A preset is only reported ready once its verified file and
                // directory entry are durable. A failure leaves the file in
                // place, reports the failure and keeps the cell empty.
                sync_published_entry(&self.cache_dir, &cached).map_err(|error| {
                    tracing::warn!(
                        preset = preset.id,
                        error = %format!("{error:#}"),
                        "the cached tokenizer preset could not be flushed"
                    );
                    DomainError::internal(
                        "the verified tokenizer file could not be flushed; retry shortly",
                    )
                })?;
                slot.activate(engine);
                return Ok(TokenizerProvisionStatus::AlreadyPresent);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    preset = preset.id,
                    error = %format!("{error:#}"),
                    "the tokenizer cache entry cannot be inspected"
                );
                return Err(DomainError::internal(
                    "the tokenizer cache entry for this preset cannot be inspected",
                )
                .into());
            }
        }

        let bytes = self
            .downloader
            .download(preset)
            .await
            .map_err(|error| DomainError::internal(error.to_string()))?;
        let engine = parse_verified_tokenizer(&bytes)
            .map_err(|error| DomainError::internal(error.to_string()))?;
        publish_cached_artifact(&self.cache_dir, preset.sha256, &bytes).map_err(|error| {
            // The publication failure never discloses the cache path.
            tracing::warn!(
                preset = preset.id,
                error = %format!("{error:#}"),
                "the verified tokenizer file could not be published"
            );
            DomainError::internal(
                "the verified tokenizer file could not be published into the cache directory",
            )
        })?;
        slot.activate(engine);
        Ok(TokenizerProvisionStatus::Provisioned)
    }
}

/// The refusal of an identifier that names no known preset.
fn unknown_preset(preset_id: &str) -> DomainError {
    DomainError::invalid_argument(format!("the tokenizer preset {preset_id} is not known"))
}

/// The outcome of one preparation.
fn response(
    preset: &TokenizerPreset,
    status: TokenizerProvisionStatus,
    ready: bool,
) -> TokenizerProvisionResponse {
    TokenizerProvisionResponse {
        preset_id: preset.id.to_string(),
        sha256: preset.sha256.to_string(),
        size_bytes: preset.size_bytes,
        status,
        ready,
        license: preset.license.to_string(),
    }
}

/// The descriptors of the known presets of this serving process.
///
/// The readiness is the readiness of this process: a preset is ready when a
/// verified engine for it is usable right now. A replica that has not
/// prepared the preset reports it as not ready, so the response never
/// promises readiness another replica holds.
pub fn preset_descriptors() -> Vec<TokenizerPresetDescriptor> {
    let slots = crate::chunking::chunking_token_artifact::global_preset_slots();
    PRESETS
        .iter()
        .map(|preset| TokenizerPresetDescriptor {
            id: preset.id.to_string(),
            label: preset.label.to_string(),
            sha256: preset.sha256.to_string(),
            size_bytes: preset.size_bytes,
            license: preset.license.to_string(),
            license_url: preset.license_url.to_string(),
            source_repo: preset.repo.to_string(),
            source_url: preset.source_url(),
            revision: preset.revision.to_string(),
            ready: slots.slot(preset.id).and_then(PresetSlot::engine).is_some(),
        })
        .collect()
}

#[cfg(test)]
#[path = "tokenizer_provisioning_tests.rs"]
mod tests;
