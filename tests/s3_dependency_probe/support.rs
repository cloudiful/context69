//! Shared fixtures for the S3 dependency-gate recovery tests (issue 702 P2).
//!
//! The S3 gate is tripped by storage failures but no ordinary storage call can
//! lift it: operations refuse to touch the backend while the gate is not closed.
//! These helpers build a `LibraryService` over a loopback-unreachable S3
//! endpoint, seed and restore the single `s3` gate row, and run a local stand-in
//! S3 that answers the bounded probe without any external service.
//!
//! Every case mutates the singleton `s3` gate row, so all cases take the one
//! `SUITE_LOCK` here. DB-backed cases run only when
//! `CONTEXT69_TEST_DATABASE_URL` is set and are skipped otherwise.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use context69::chunking::ChunkingConfig;
use context69::config::{FileLibraryConfig, S3StorageConfig};
use context69::db::Database;
use context69::library_store::{DependencyGateRecord, LibraryStore};
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::settings::SettingsService;
use context69_extraction::{
    ExtractionDependencies, ExtractionPublication, ExtractionPublisher, ExtractionReadiness,
    ExtractionService,
};
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// No-op stand-in for the translation/extraction callbacks LibraryService requires.
struct NoopCallbacks;

#[async_trait]
impl TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

#[async_trait]
impl ExtractionPublisher for NoopCallbacks {
    async fn publish(&self, _publication: &ExtractionPublication<'_>) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ExtractionReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// Serializes the cases that mutate the singleton `s3` gate row.
pub(crate) static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn connect_db() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

/// A service whose storage backend is S3 with a loopback-unreachable endpoint,
/// built through the same constructor the application uses.
pub(crate) async fn build_library_service(
    db: &Database,
    s3: Option<S3StorageConfig>,
) -> LibraryService {
    // The production binary installs the process-wide HTTP transport before
    // startup; an integration test must do the same for an S3 operator to make a
    // real network attempt. First-installed-wins, safe to call repeatedly.
    opendal::install_default();
    let storage_root = std::env::temp_dir().join(format!("context69-s3-probe-{}", Uuid::new_v4()));
    let settings = SettingsService::new(db.clone());
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    let extraction = ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            file_library: FileLibraryConfig {
                storage_root,
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "test".to_string(),
        },
        settings,
        translation,
        extraction,
    )
    .await
    .expect("build library service")
}

pub(crate) fn unreachable_s3() -> S3StorageConfig {
    S3StorageConfig {
        endpoint: "http://127.0.0.1:1".to_string(),
        region: "context69-test".to_string(),
        bucket: "context69-test".to_string(),
        prefix: "library".to_string(),
        path_style: true,
        access_key: "AKIACONTEXT69TESTONLY".to_string(),
        secret_key: "synthetic-s3-secret".to_string(),
    }
}

/// The same loopback-unreachable credentials, pointed at a local stand-in.
pub(crate) fn s3_pointed_at(endpoint: &str) -> S3StorageConfig {
    S3StorageConfig {
        endpoint: endpoint.to_string(),
        ..unreachable_s3()
    }
}

/// Seed the `s3` gate into one exact state.
pub(crate) async fn seed_s3_gate(
    db: &Database,
    state: &str,
    failure_count: i32,
    next_probe_at: Option<chrono::DateTime<Utc>>,
    last_error: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO context69.library_dependency_gates \
             (dependency_key, state, failure_count, next_probe_at, last_error) \
         VALUES ('s3', $1, $2, $3, $4) \
         ON CONFLICT (dependency_key) DO UPDATE SET \
             state = EXCLUDED.state, failure_count = EXCLUDED.failure_count, \
             next_probe_at = EXCLUDED.next_probe_at, last_error = EXCLUDED.last_error, \
             probe_lease_token = NULL, probe_lease_expires_at = NULL, updated_at = now()",
    )
    .bind(state)
    .bind(failure_count)
    .bind(next_probe_at)
    .bind(last_error)
    .execute(db.pool())
    .await
    .expect("seed s3 gate");
}

/// Restore the `s3` gate to its healthy default so no case leaks state.
pub(crate) async fn restore_s3_gate(db: &Database) {
    sqlx::query(
        "UPDATE context69.library_dependency_gates SET state = 'closed', failure_count = 0, \
         next_probe_at = NULL, last_error = NULL, probe_lease_token = NULL, \
         probe_lease_expires_at = NULL, last_success_at = NULL, updated_at = now() \
         WHERE dependency_key = 's3'",
    )
    .execute(db.pool())
    .await
    .expect("restore s3 gate");
}

pub(crate) async fn s3_gate(store: &LibraryStore) -> DependencyGateRecord {
    store
        .list_dependency_gates()
        .await
        .expect("list dependency gates")
        .into_iter()
        .find(|gate| gate.dependency_key == "s3")
        .expect("s3 gate row")
}

/// A loopback S3 stand-in that answers the bounded `check()` (`ListObjectsV2`)
/// with one empty page, so a probe can succeed without an external service.
pub(crate) struct FakeS3 {
    pub(crate) endpoint: String,
    pub(crate) requests: Arc<AtomicUsize>,
    pub(crate) stop: CancellationToken,
}

pub(crate) async fn start_fake_s3(response_delay: std::time::Duration) -> FakeS3 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the fake s3 listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("fake s3 address"));
    let requests = Arc::new(AtomicUsize::new(0));
    let stop = CancellationToken::new();
    let (server_requests, server_stop) = (requests.clone(), stop.clone());
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = server_stop.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((mut socket, _)) = accepted else { break };
            let served = server_requests.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => head.extend_from_slice(&chunk[..read]),
                    }
                }
                served.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // The delay keeps the reserved gate observable as `half_open`.
                tokio::time::sleep(response_delay).await;
                let body = concat!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                    "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
                    "<Name>context69-test</Name><KeyCount>0</KeyCount>",
                    "<IsTruncated>false</IsTruncated></ListBucketResult>"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/xml\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    FakeS3 {
        endpoint,
        requests,
        stop,
    }
}
