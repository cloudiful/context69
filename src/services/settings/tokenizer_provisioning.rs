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

    /// The writable directory the prepared presets are cached in.
    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }

    /// The descriptors of the known presets, with their readiness.
    ///
    /// The readiness is the readiness of this serving process: a preset is
    /// ready when a verified engine for it is usable right now.
    pub fn descriptors(&self) -> Vec<TokenizerPresetDescriptor> {
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
                ready: self.is_ready(preset.id),
            })
            .collect()
    }

    /// Whether this process holds a verified engine for one preset.
    pub fn is_ready(&self, preset_id: &str) -> bool {
        self.slots
            .slot(preset_id)
            .and_then(PresetSlot::engine)
            .is_some()
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
        let preset = preset_by_id(request.preset_id.trim()).ok_or_else(|| {
            DomainError::invalid_argument(format!(
                "the tokenizer preset {} is not known",
                request.preset_id.trim()
            ))
        })?;
        let slot = self.slots.slot(preset.id).ok_or_else(|| {
            DomainError::invalid_argument(format!(
                "the tokenizer preset {} is not known",
                preset.id
            ))
        })?;

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
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
        time::SystemTime,
    };

    use super::*;
    use crate::{
        chunking::chunking_token_artifact::{TokenizerCatalog, test_tokenizer_json},
        tokenizer_artifact_download::{
            TokenizerArtifactDownloader,
            test_support::{self, pinned_test_preset},
        },
    };

    fn cache_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "context69-tokenizer-provisioning-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create the cache directory");
        directory
    }

    fn test_preset(bytes: &[u8]) -> &'static TokenizerPreset {
        Box::leak(Box::new(pinned_test_preset(bytes)))
    }

    fn provisioning(
        cache_dir: &Path,
        downloader: TokenizerArtifactDownloader,
    ) -> TokenizerProvisioning {
        TokenizerProvisioning::with_downloader(
            cache_dir.to_path_buf(),
            Arc::new(PresetSlots::new()),
            downloader,
        )
    }

    async fn loopback_downloader(
        server: &test_support::LoopbackServer,
    ) -> TokenizerArtifactDownloader {
        TokenizerArtifactDownloader::with_url_override(&server.url)
            .expect("build the loopback downloader")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_cache_sync_failure_must_not_activate_the_slot() {
        use std::os::unix::fs::PermissionsExt;

        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("review-sync-failure");
        let path = directory.join(format!("{}.json", preset.sha256));
        std::fs::write(&path, &bytes).expect("write verified cache");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o300))
            .expect("deny directory read while retaining search");
        let result = provisioning.prepare(preset, &slot).await;
        let ready = slot.engine().is_some();
        let retry = provisioning.prepare(preset, &slot).await;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .expect("restore directory permissions");
        let recovered = provisioning.prepare(preset, &slot).await;
        let recovered_ready = slot.engine().is_some();
        let requests = server.request_count();
        std::fs::remove_dir_all(&directory).expect("remove own cache");
        assert!(result.is_err(), "directory sync must fail");
        assert_eq!(
            recovered.expect("recovery"),
            TokenizerProvisionStatus::AlreadyPresent
        );
        assert!(recovered_ready);
        assert_eq!(requests, 0);
        assert!(
            !ready && retry.is_err(),
            "failed sync must not activate the slot or let retry bypass sync: ready={ready}, retry={retry:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_service_logs_full_cause_and_returns_sanitized_error() {
        use std::os::unix::fs::PermissionsExt;

        struct LogWriter(Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for LogWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("log lock").write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("review-service-log");
        let path = directory.join(format!("{}.json", preset.sha256));
        std::fs::write(&path, bytes).expect("write cache");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
            .expect("deny cache file reads");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();
        let logs = Arc::new(std::sync::Mutex::new(Vec::new()));
        let output = Arc::clone(&logs);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || LogWriter(Arc::clone(&output)))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let result = provisioning.prepare(preset, &slot).await;
        drop(guard);
        let log = String::from_utf8(logs.lock().expect("log lock").clone()).expect("utf8 log");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("restore cache permissions");
        std::fs::remove_dir_all(&directory).expect("remove own cache");
        let error = result.expect_err("invalid cache");
        assert!(log.contains("read the cached tokenizer preset"), "{log}");
        assert!(log.contains("Permission denied"), "{log}");
        assert!(log.contains(path.to_str().expect("path")), "{log}");
        assert!(
            !error
                .to_string()
                .contains(directory.to_str().expect("path"))
        );
        assert!(slot.engine().is_none());
        assert_eq!(server.request_count(), 0);
    }

    #[tokio::test]
    async fn a_prepared_preset_is_ready_and_a_repeat_is_idempotent() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("idempotent");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();

        let first = provisioning
            .prepare(preset, &slot)
            .await
            .expect("prepare the preset");
        assert_eq!(first, TokenizerProvisionStatus::Provisioned);
        assert!(slot.engine().is_some(), "the verified engine is active");
        assert_eq!(
            std::fs::read(directory.join(format!("{}.json", preset.sha256)))
                .expect("read the cached file"),
            bytes
        );

        let second = provisioning
            .prepare(preset, &slot)
            .await
            .expect("prepare the preset again");
        assert_eq!(second, TokenizerProvisionStatus::AlreadyReady);
        assert_eq!(
            server.request_count(),
            1,
            "one retrieval prepares the preset"
        );

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn a_cached_preset_activates_after_a_restart_without_a_retrieval() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("restart");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let serialized = PresetSlot::for_test();
        provisioning
            .prepare(preset, &serialized)
            .await
            .expect("prepare the preset");
        assert_eq!(server.request_count(), 1);

        // A new process keeps only the verified cache file, not the cell.
        let restarted_slot = PresetSlot::for_test();
        crate::chunking::chunking_token_artifact::activate_cached_preset(
            &restarted_slot,
            &directory.join(format!("{}.json", preset.sha256)),
            preset,
        )
        .expect("activate the cached preset");
        assert!(
            restarted_slot.engine().is_some(),
            "the cache activates offline"
        );
        assert_eq!(
            server.request_count(),
            1,
            "restart activation never retrieves the file again"
        );

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn a_retrieval_that_does_not_verify_publishes_nothing() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let mut other = bytes.clone();
        let last = other.len() - 2;
        other[last] = if other[last] == b' ' { b'!' } else { b' ' };
        let server = test_support::serve_bytes(other).await;
        let directory = cache_directory("mismatch");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();

        provisioning
            .prepare(preset, &slot)
            .await
            .expect_err("a mismatched file is refused");

        assert!(slot.engine().is_none(), "no engine is published");
        assert!(
            !directory.join(format!("{}.json", preset.sha256)).exists(),
            "no cache file is published"
        );

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn concurrent_requests_prepare_the_preset_once() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("concurrent");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();

        let (first, second) = tokio::join!(
            provisioning.prepare(preset, &slot),
            provisioning.prepare(preset, &slot),
        );
        let statuses = [
            first.expect("the first request prepares the preset"),
            second.expect("the second request observes the preparation"),
        ];
        assert!(statuses.contains(&TokenizerProvisionStatus::Provisioned));
        assert!(statuses.contains(&TokenizerProvisionStatus::AlreadyReady));
        assert_eq!(
            server.request_count(),
            1,
            "one retrieval serves both requests"
        );
        assert!(slot.engine().is_some());

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn a_request_that_cannot_start_is_refused_within_the_bounded_wait() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("deadline");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await)
            .with_wait(std::time::Duration::from_millis(50));
        let slot = PresetSlot::for_test();

        let _held = slot.lock().lock().await;
        let error = provisioning
            .prepare(preset, &slot)
            .await
            .expect_err("a preparation that cannot start is refused");
        assert!(error.to_string().contains("still running"));
        assert_eq!(server.request_count(), 0, "no retrieval starts");

        drop(_held);
        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn an_unknown_identifier_is_refused_before_any_work() {
        let directory = cache_directory("unknown");
        let provisioning =
            TokenizerProvisioning::with_slots(directory.clone(), Arc::new(PresetSlots::new()))
                .expect("build the provisioning");

        let error = provisioning
            .provision(&TokenizerProvisionRequest {
                preset_id: "not_a_known_preset".to_string(),
            })
            .await
            .expect_err("an unknown identifier is refused");
        assert!(error.to_string().contains("is not known"));

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn a_missing_operator_directory_still_prepares_the_preset() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("missing-operator-directory");
        let operator_directory = directory.join("missing-artifacts");
        let cache_dir = directory.join("cache");
        let slots = Arc::new(PresetSlots::new());
        // A fresh installation has no mounted artifact directory; the catalog
        // must still carry the configured preset cache.
        let catalog = TokenizerCatalog::load_with_presets(
            &operator_directory,
            Arc::clone(&slots),
            Some(cache_dir.clone()),
        )
        .expect("load the catalog without an operator directory");
        assert_eq!(catalog.preset_cache_dir(), Some(cache_dir.as_path()));
        let provisioning = TokenizerProvisioning::with_downloader(
            cache_dir.clone(),
            slots,
            loopback_downloader(&server).await,
        );
        let slot = PresetSlot::for_test();

        let status = provisioning
            .prepare(preset, &slot)
            .await
            .expect("prepare through the configured cache");

        assert_eq!(status, TokenizerProvisionStatus::Provisioned);
        assert!(slot.engine().is_some());
        assert!(cache_dir.join(format!("{}.json", preset.sha256)).is_file());

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn a_verified_cached_file_is_activated_and_flushed_without_a_retrieval() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("verified-cached-entry");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let path = directory.join(format!("{}.json", preset.sha256));
        crate::tokenizer_artifact_download::publish_cached_artifact(
            &directory,
            preset.sha256,
            &bytes,
        )
        .expect("publish the verified cache file");
        let slot = PresetSlot::for_test();

        let status = provisioning
            .prepare(preset, &slot)
            .await
            .expect("activate the verified cache file");

        assert_eq!(status, TokenizerProvisionStatus::AlreadyPresent);
        assert!(slot.engine().is_some(), "the verified entry is active");
        assert_eq!(
            server.request_count(),
            0,
            "a verified cache file needs no retrieval"
        );
        assert_eq!(
            std::fs::read(&path).expect("read the cache file"),
            bytes,
            "the verified entry is unchanged"
        );

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[tokio::test]
    async fn an_unusable_cached_file_is_refused_and_left_untouched() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("corrupt-cache-entry");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();
        let path = directory.join(format!("{}.json", preset.sha256));
        let corrupt = b"{\"version\":\"corrupt\"}";
        std::fs::write(&path, corrupt).expect("write the corrupt cache entry");

        let error = provisioning
            .prepare(preset, &slot)
            .await
            .expect_err("an unusable cache entry is refused");

        assert!(error.to_string().contains("is not usable"));
        assert!(
            !error.to_string().contains(&directory.display().to_string()),
            "the response never discloses the cache path"
        );
        assert_eq!(
            std::fs::read(&path).expect("read the untouched entry"),
            corrupt,
            "the operator's file is preserved"
        );
        assert!(slot.engine().is_none(), "nothing is activated");
        assert_eq!(server.request_count(), 0, "no retrieval replaces the entry");

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_dangling_symlink_at_the_cache_path_is_refused_and_left_untouched() {
        let bytes = test_tokenizer_json(false);
        let preset = test_preset(&bytes);
        let server = test_support::serve_bytes(bytes.clone()).await;
        let directory = cache_directory("dangling-cache-entry");
        let provisioning = provisioning(&directory, loopback_downloader(&server).await);
        let slot = PresetSlot::for_test();
        let path = directory.join(format!("{}.json", preset.sha256));
        let missing = directory.join("absent.json");
        std::os::unix::fs::symlink(&missing, &path).expect("create the dangling symlink");

        provisioning
            .prepare(preset, &slot)
            .await
            .expect_err("a dangling symlink is refused");

        let metadata = std::fs::symlink_metadata(&path).expect("the symlink is untouched");
        assert!(
            metadata.file_type().is_symlink(),
            "the symlink stays a symlink"
        );
        assert_eq!(
            std::fs::read_link(&path).expect("read the link"),
            missing,
            "the link target is unchanged"
        );
        assert!(slot.engine().is_none(), "nothing is activated");
        assert_eq!(server.request_count(), 0, "no retrieval replaces the entry");

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[test]
    fn the_descriptors_report_the_pinned_license_and_a_process_local_readiness() {
        let descriptors = preset_descriptors();
        let jina = descriptors
            .iter()
            .find(|descriptor| {
                descriptor.id == crate::tokenizer_presets::JINA_V5_SMALL_RETRIEVAL_R6856E76
            })
            .expect("the Jina preset is described");
        assert_eq!(jina.license, "cc-by-nc-4.0");
        assert_eq!(
            jina.license_url,
            "https://creativecommons.org/licenses/by-nc/4.0/"
        );
        assert_eq!(jina.size_bytes, 11_422_654);
    }
}
