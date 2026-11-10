//! Cases for the explicit preparation of one known tokenizer preset.
//!
//! Every case serves its own bytes through the loopback transport in
//! `tokenizer_artifact_download::test_support` and keeps its preset cell out of
//! the process-wide cells.

use std::{path::Path, sync::Arc};

use super::*;
use crate::{
    chunking::chunking_token_artifact::{TokenizerCatalog, test_tokenizer_json},
    tokenizer_artifact_download::{
        TokenizerArtifactDownloader,
        test_support::{self, pinned_test_preset},
    },
};

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

async fn loopback_downloader(server: &test_support::LoopbackServer) -> TokenizerArtifactDownloader {
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
    let directory = test_support::test_directory("provisioning-review-sync-failure");
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
    let directory = test_support::test_directory("provisioning-review-service-log");
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
    let directory = test_support::test_directory("provisioning-idempotent");
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
    let directory = test_support::test_directory("provisioning-restart");
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
async fn a_verified_catalog_entry_is_ready_without_any_retrieval() {
    let bytes = test_tokenizer_json(false);
    let preset = test_preset(&bytes);
    let server = test_support::serve_bytes(bytes.clone()).await;
    let directory = test_support::test_directory("provisioning-catalog-ready");
    let artifacts = test_support::test_directory("provisioning-catalog-artifacts");
    std::fs::write(artifacts.join(format!("{}.json", preset.sha256)), &bytes)
        .expect("write the mounted artifact");
    let catalog =
        TokenizerCatalog::load_with_presets(&artifacts, Arc::new(PresetSlots::new()), None)
            .expect("load the verified catalog");
    let provisioning =
        provisioning(&directory, loopback_downloader(&server).await).with_catalog(catalog);
    let slot = PresetSlot::for_test();

    assert!(
        provisioning.preset_ready(preset, &slot),
        "a verified catalog entry makes the preset ready"
    );
    let status = provisioning
        .prepare(preset, &slot)
        .await
        .expect("prepare the bundled preset");
    assert_eq!(status, TokenizerProvisionStatus::AlreadyReady);
    assert_eq!(server.request_count(), 0, "no retrieval is started");
    assert!(
        !directory.join(format!("{}.json", preset.sha256)).exists(),
        "nothing is written to the writable preset cache"
    );

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    std::fs::remove_dir_all(&artifacts).expect("remove the artifact directory");
}

#[tokio::test]
async fn a_retrieval_that_does_not_verify_publishes_nothing() {
    let bytes = test_tokenizer_json(false);
    let preset = test_preset(&bytes);
    let mut other = bytes.clone();
    let last = other.len() - 2;
    other[last] = if other[last] == b' ' { b'!' } else { b' ' };
    let server = test_support::serve_bytes(other).await;
    let directory = test_support::test_directory("provisioning-mismatch");
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
    let directory = test_support::test_directory("provisioning-concurrent");
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
    let directory = test_support::test_directory("provisioning-deadline");
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
    let directory = test_support::test_directory("provisioning-unknown");
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
    let directory = test_support::test_directory("provisioning-missing-operator-directory");
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
    let directory = test_support::test_directory("provisioning-verified-cached-entry");
    let provisioning = provisioning(&directory, loopback_downloader(&server).await);
    let path = directory.join(format!("{}.json", preset.sha256));
    crate::tokenizer_artifact_download::publish_cached_artifact(&directory, preset.sha256, &bytes)
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
    let directory = test_support::test_directory("provisioning-corrupt-cache-entry");
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
    let directory = test_support::test_directory("provisioning-dangling-cache-entry");
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
    let gemma = descriptors
        .iter()
        .find(|descriptor| descriptor.id == crate::tokenizer_presets::EMBEDDINGGEMMA2_R_F9972AC)
        .expect("the EmbeddingGemma 2 preset is described");
    assert_eq!(gemma.label, "EmbeddingGemma 2");
    assert_eq!(gemma.license, "apache-2.0");
    assert_eq!(
        gemma.license_url,
        "https://www.apache.org/licenses/LICENSE-2.0"
    );
    assert_eq!(
        gemma.sha256,
        "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4"
    );
    assert_eq!(gemma.size_bytes, 32_170_510);
    assert_eq!(gemma.source_repo, "google/embeddinggemma-2");
    assert_eq!(gemma.revision, "f9972ac22e696a105c482202646979e33e4b5a1e");
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.id != "jina_v5_small_retrieval_r6856e76"),
        "the retired preset is not described"
    );
}
