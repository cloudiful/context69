//! Rowless staging sweep (issue 667 Phase 2C).
//!
//! A streamed upload can die after its bytes land but before its catalog row
//! commits, so the catalog sweep never sees those bytes. List `staging/`
//! directly, keep only stale in-size keys, and delete through the S3 gate.

use std::future::Future;
use std::time::SystemTime;

use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::warn;
use uuid::Uuid;

use super::LibraryService;
use super::object_storage::{STAGING_KEY_PREFIX, StagingEntry};

/// One bounded lister page per `start_after` step.
pub(crate) const ROWLESS_STAGING_SWEEP_PAGE_SIZE: usize = 100;

/// Maximum delete attempts one drain may make.
pub(crate) const ROWLESS_STAGING_SWEEP_BATCH_SIZE: usize = 50;

/// Maximum entries one drain may inspect, so a prefix full of fresh, malformed,
/// or oversized keys cannot hold the drain forever.
pub(crate) const ROWLESS_STAGING_SWEEP_SCAN_BUDGET: usize = 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RowlessStagingSweepSummary {
    pub scanned: usize,
    pub deleted: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Parse a strictly `staging/<signed i64 group>/<uuid>` file key. Directory
/// entries, nested or malformed keys, and `objects/` keys are never candidates.
fn staging_key_identity(key: &str) -> Option<(i64, Uuid)> {
    let rest = key.strip_prefix(STAGING_KEY_PREFIX)?;
    let (group, token) = rest.split_once('/')?;
    if token.contains('/') {
        return None;
    }
    Some((group.parse().ok()?, Uuid::parse_str(token).ok()?))
}

/// A key is deletable only when canonical, strictly older than `cutoff`, and no
/// larger than `max_bytes`.
fn staging_entry_is_deletable(entry: &StagingEntry, cutoff: SystemTime, max_bytes: u64) -> bool {
    if staging_key_identity(&entry.key).is_none() {
        return false;
    }
    match entry.modified {
        Some(modified) => modified < cutoff && entry.size_bytes <= max_bytes,
        None => false,
    }
}

/// Drive one bounded rowless sweep within a single drain. `list_page` pages
/// strictly after an owned cursor; a backend that ignores the cursor repeats
/// the page, and the loop stops instead of re-listing forever.
async fn run_rowless_staging_sweep<L, LF, D, DF>(
    mut list_page: L,
    mut delete_object: D,
    cutoff: SystemTime,
    max_bytes: u64,
    page_size: usize,
    batch_size: usize,
    scan_budget: usize,
) -> Result<RowlessStagingSweepSummary>
where
    L: FnMut(Option<String>, usize) -> LF,
    LF: Future<Output = Result<Vec<StagingEntry>>>,
    D: FnMut(String) -> DF,
    DF: Future<Output = Result<()>>,
{
    let page_size = page_size.max(1);
    let batch_size = batch_size.max(1);
    let scan_budget = scan_budget.max(1);
    let mut summary = RowlessStagingSweepSummary::default();
    let mut cursor: Option<String> = None;
    loop {
        let page = list_page(cursor.clone(), page_size).await?;
        let Some(last_key) = page.last().map(|entry| entry.key.clone()) else {
            break;
        };
        let page_len = page.len();
        for entry in page {
            if summary.scanned >= scan_budget || summary.deleted + summary.failed >= batch_size {
                break;
            }
            summary.scanned += 1;
            if !staging_entry_is_deletable(&entry, cutoff, max_bytes) {
                summary.skipped += 1;
                continue;
            }
            let object_key = entry.key;
            match delete_object(object_key.clone()).await {
                Ok(()) => summary.deleted += 1,
                Err(error) => {
                    summary.failed += 1;
                    warn!(%object_key, %error, "rowless staging delete failed; a later drain retries");
                }
            }
        }
        if summary.scanned >= scan_budget || summary.deleted + summary.failed >= batch_size {
            break;
        }
        if cursor.as_deref() == Some(last_key.as_str()) || page_len < page_size {
            break;
        }
        cursor = Some(last_key);
    }
    Ok(summary)
}

impl LibraryService {
    /// Sweep staging bytes with no catalog row. A staging key is never a catalog
    /// `object_key`, so no reference check is needed and no row is removed.
    pub(crate) async fn sweep_rowless_staging_objects(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<RowlessStagingSweepSummary> {
        run_rowless_staging_sweep(
            |start_after: Option<String>, limit| {
                let library = self;
                async move {
                    library
                        .list_active_staging_page(start_after.as_deref(), limit)
                        .await
                }
            },
            |key: String| {
                let library = self;
                async move { library.delete_active_storage(&key).await }
            },
            SystemTime::from(cutoff),
            self.max_upload_size_bytes as u64,
            ROWLESS_STAGING_SWEEP_PAGE_SIZE,
            ROWLESS_STAGING_SWEEP_BATCH_SIZE,
            ROWLESS_STAGING_SWEEP_SCAN_BUDGET,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    use anyhow::{Result, anyhow};
    use uuid::Uuid;

    use crate::config::FileLibraryConfig;
    use crate::services::library::object_storage::{
        LibraryObjectStorage, StagingEntry, content_object_key, staging_object_key,
    };

    use super::run_rowless_staging_sweep;

    fn stale_time() -> SystemTime {
        SystemTime::now() - Duration::from_secs(25 * 60 * 60)
    }

    fn fresh_time() -> SystemTime {
        SystemTime::now() - Duration::from_secs(60 * 60)
    }

    fn cutoff_time() -> SystemTime {
        SystemTime::now() - Duration::from_secs(24 * 60 * 60)
    }

    fn key(n: u32) -> String {
        format!("staging/7/aaaaaaaa-0000-0000-0000-{n:012}")
    }

    fn entry(key: impl Into<String>, size: u64, modified: Option<SystemTime>) -> StagingEntry {
        StagingEntry {
            key: key.into(),
            size_bytes: size,
            modified,
        }
    }

    /// Cursor paging over a key-sorted listing, mirroring S3 `start_after`.
    fn fake_lister(
        mut entries: Vec<StagingEntry>,
    ) -> impl FnMut(Option<String>, usize) -> std::future::Ready<Result<Vec<StagingEntry>>> {
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        move |start_after, limit| {
            let page = entries
                .iter()
                .filter(|entry| {
                    start_after
                        .as_deref()
                        .is_none_or(|c| entry.key.as_str() > c)
                })
                .take(limit)
                .cloned()
                .collect();
            std::future::ready(Ok(page))
        }
    }

    fn deleted_log() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn recording_delete(
        recorder: Arc<Mutex<Vec<String>>>,
    ) -> impl FnMut(String) -> std::future::Ready<Result<()>> {
        move |key: String| {
            recorder.lock().unwrap().push(key);
            std::future::ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn deletes_only_stale_valid_sized_files() {
        let token = Uuid::new_v4();
        // `staging/-3/...` sorts before `staging/7/...`, so deletion order is
        // deterministic over the key-sorted fake listing.
        let negative = format!("staging/-3/{token}");
        let entries = vec![
            entry(key(1), 10, Some(stale_time())),
            entry(key(2), 10, Some(fresh_time())),
            entry(key(3), 10, None),
            entry(key(4), 101, Some(stale_time())),
            entry(format!("staging/notanumber/{token}"), 1, Some(stale_time())),
            entry(format!("staging/7/8/{token}"), 1, Some(stale_time())),
            entry("staging/7/", 0, Some(stale_time())),
            entry("objects/7/deadbeef", 1, Some(stale_time())),
            entry(negative.clone(), 1, Some(stale_time())),
        ];
        let deleted = deleted_log();
        let delete = recording_delete(Arc::clone(&deleted));
        let summary = run_rowless_staging_sweep(
            fake_lister(entries),
            delete,
            cutoff_time(),
            100,
            100,
            100,
            100,
        )
        .await
        .unwrap();

        assert_eq!(
            (summary.deleted, summary.failed, summary.skipped),
            (2, 0, 7)
        );
        assert_eq!(deleted.lock().unwrap().as_slice(), [negative, key(1)]);
    }

    #[tokio::test]
    async fn paging_budgets_and_failures_bound_each_drain() {
        // A fresh head proves `start_after` paging reaches the stale entry.
        let mut entries: Vec<_> = (1..=4)
            .map(|n| entry(key(n), 0, Some(fresh_time())))
            .collect();
        entries.push(entry(key(9), 1, Some(stale_time())));
        let deleted = deleted_log();
        let delete = recording_delete(Arc::clone(&deleted));
        let paged =
            run_rowless_staging_sweep(fake_lister(entries), delete, cutoff_time(), 100, 2, 10, 10)
                .await
                .unwrap();
        assert_eq!((paged.deleted, paged.scanned), (1, 5));
        assert_eq!(deleted.lock().unwrap().as_slice(), [key(9)]);

        // Batch budget caps one drain; a per-key delete failure is counted,
        // never blocks the drain, and is left for the next one to retry.
        let entries: Vec<_> = (1..=5)
            .map(|n| entry(key(n), 1, Some(stale_time())))
            .collect();
        let deleted = deleted_log();
        let delete = recording_delete(Arc::clone(&deleted));
        let batch = run_rowless_staging_sweep(
            fake_lister(entries.clone()),
            delete,
            cutoff_time(),
            100,
            10,
            2,
            10,
        )
        .await
        .unwrap();
        assert_eq!((batch.deleted, batch.scanned), (2, 2));
        assert_eq!(deleted.lock().unwrap().len(), 2);

        let fail_key = key(1);
        let delete = recording_delete(deleted_log());
        let failing = {
            let mut inner = delete;
            move |key: String| match key == fail_key {
                true => std::future::ready(Err(anyhow!("injected delete failure"))),
                false => inner(key),
            }
        };
        let capped =
            run_rowless_staging_sweep(fake_lister(entries), failing, cutoff_time(), 100, 10, 10, 1)
                .await
                .unwrap();
        assert_eq!((capped.deleted, capped.failed, capped.scanned), (0, 1, 1));
    }

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("context69-rowless-{}", Uuid::new_v4()))
    }

    fn local_config(root: &Path) -> FileLibraryConfig {
        FileLibraryConfig {
            storage_root: root.to_path_buf(),
            max_upload_size_mb: 1,
            max_upload_request_size_mb: 1,
            ingest_concurrency: 1,
            url_import_concurrency: 1,
            url_import_min_interval_ms: 1000,
            trusted_proxy_enabled: false,
            s3: None,
        }
    }

    fn set_modified(root: &Path, key: &str, modified: SystemTime) {
        let file = std::fs::File::options()
            .write(true)
            .open(root.join(key))
            .unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[tokio::test]
    async fn local_backend_sweeps_only_stale_in_size_staging_files() {
        let root = temp_root();
        let storage = LibraryObjectStorage::from_config(&local_config(&root)).unwrap();
        let stale_a = staging_object_key(5, Uuid::new_v4());
        let stale_b = staging_object_key(5, Uuid::new_v4());
        let fresh = staging_object_key(5, Uuid::new_v4());
        let oversized = staging_object_key(5, Uuid::new_v4());
        let malformed = "staging/5/not-a-uuid".to_string();
        let content = content_object_key(5, &"a".repeat(64));
        for (key, len, modified) in [
            (stale_a.as_str(), 5usize, stale_time()),
            (stale_b.as_str(), 5, stale_time()),
            (fresh.as_str(), 5, fresh_time()),
            (oversized.as_str(), 9, stale_time()),
            (malformed.as_str(), 1, stale_time()),
            (content.as_str(), 1, stale_time()),
        ] {
            storage.write(key, vec![b'x'; len].into()).await.unwrap();
            set_modified(&root, key, modified);
        }
        let directory = root.join("staging/empty-group");
        std::fs::create_dir_all(&directory).unwrap();

        let list = |start_after: Option<String>, limit: usize| {
            let storage = storage.clone();
            async move {
                storage
                    .list_staging_page(start_after.as_deref(), limit)
                    .await
            }
        };
        let delete = |key: String| {
            let storage = storage.clone();
            async move { storage.delete(&key).await }
        };

        // Batch size 1 forces one delete per drain; the second drain clears the
        // rest, so the sweep converges without a persisted cursor and never
        // touches the fresh, oversized, malformed, directory, or `objects/` key.
        let first = run_rowless_staging_sweep(&list, &delete, cutoff_time(), 8, 100, 1, 100)
            .await
            .unwrap();
        assert_eq!(
            first.deleted, 1,
            "batch budget caps the drain at one delete"
        );
        let second = run_rowless_staging_sweep(list, delete, cutoff_time(), 8, 100, 1, 100)
            .await
            .unwrap();
        assert_eq!(second.deleted, 1);
        assert!(!root.join(&stale_a).exists() && !root.join(&stale_b).exists());
        for survivor in [&fresh, &oversized, &malformed, &content] {
            assert!(root.join(survivor).exists(), "must survive: {survivor}");
        }
        assert!(directory.exists(), "a staging directory must survive");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
