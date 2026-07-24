use anyhow::{Context, Result};
use db_init::{DbInitOptions, connect_pool, run_migrations};
use sqlx::PgPool;
use uuid::Uuid;

use crate::contracts::{SearchMode, SyncOutcome};
use crate::domain::SyncCheckpoint;

mod auth;
mod docling_settings;
mod documents;
mod internal_secrets;
mod metadata_indexes;
mod namespaces;
mod personal_access_tokens;
mod rows;
mod runtime_settings;
mod search_cache;
mod search_settings;
mod source_connections;
mod sync_runs;
mod tasks;
mod translations;
mod vector_index_state;

pub use context69_db_schema::MIGRATOR;
pub(crate) use metadata_indexes::metadata_value_rows;
pub use metadata_indexes::{NewMetadataIndex, StoredMetadataIndex};
pub use personal_access_tokens::{NewPersonalAccessToken, PersonalAccessTokenRecord};
use rows::*;
pub use tasks::{StoredTask, StoredTaskItem, StoredTaskPayload};
pub use vector_index_state::VectorIndexState;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct RunHandle {
    pub id: i64,
    pub source_key: String,
}

#[derive(Debug, Clone)]
pub struct UpsertedDocument {
    pub document_id: i64,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct StoredDoclingSettings {
    pub base_url: String,
    pub timeout_secs: u64,
    pub poll_interval_secs: u64,
    pub task_timeout_secs: u64,
    pub pdf_backend: Option<String>,
    pub images_scale: Option<f64>,
    pub image_export_mode: Option<String>,
    pub do_ocr: bool,
    pub force_ocr: bool,
    pub ocr_engine: Option<String>,
    pub ocr_lang: Vec<String>,
    pub do_code_enrichment: bool,
    pub do_formula_enrichment: bool,
    pub do_picture_description: bool,
    pub openai_base_url: Option<String>,
    pub api_key: Option<String>,
    pub vlm_pipeline_model: Option<String>,
    pub picture_description_model: Option<String>,
    pub code_formula_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredSearchSettings {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    pub api_key: Option<String>,
}

pub fn default_search_settings() -> StoredSearchSettings {
    StoredSearchSettings {
        mode: SearchMode::Hybrid,
        rerank_enabled: true,
        rerank_base_url: "https://openrouter.ai/api/v1".to_string(),
        rerank_model: "cohere/rerank-4-fast".to_string(),
        candidate_limit: 40,
        timeout_secs: 10,
        api_key: None,
    }
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeQdrantSettings {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeEmbeddingSettings {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub dimensions: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeSchedulerSettings {
    pub interval_secs: u64,
    pub run_on_start: bool,
    pub max_concurrency: usize,
    pub job_id: String,
    pub valkey_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeChunkingSettings {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeFileLibrarySettings {
    pub storage_root: String,
    pub max_upload_size_mb: usize,
    pub max_upload_request_size_mb: usize,
    pub ingest_concurrency: usize,
    pub pdf_pages_per_task: u32,
    pub url_import_concurrency: usize,
    pub url_import_min_interval_ms: u64,
    pub trusted_proxy_enabled: bool,
    pub s3: Option<StoredRuntimeS3Settings>,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeS3Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Debug, Clone)]
pub struct StoredSourceConnection {
    pub name: String,
    pub database_url: String,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeSettings {
    pub qdrant: StoredRuntimeQdrantSettings,
    pub embedding: StoredRuntimeEmbeddingSettings,
    pub scheduler: StoredRuntimeSchedulerSettings,
    pub chunking: StoredRuntimeChunkingSettings,
    pub file_library: StoredRuntimeFileLibrarySettings,
}

#[derive(Debug, Clone)]
pub struct StoredRerankItemScore {
    pub rerank_model: String,
    pub query_hash: String,
    pub query_text_trimmed: String,
    pub chunk_id: Uuid,
    pub score: f32,
}

impl Database {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = connect_pool(
            url,
            DbInitOptions {
                max_connections: 10,
            },
        )
        .await
        .context("failed to connect app_db pool")?;
        run_migrations(&pool, &MIGRATOR)
            .await
            .context("failed to run app_db migrations")?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn ping(&self) -> Result<()> {
        let _ = sqlx::query_file_scalar!("src/sql/db/ping.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }
}
