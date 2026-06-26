use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::contracts::{SearchMode, SourceOriginStatusKind, SourceStatus, SyncOutcome};
use crate::domain::SyncCheckpoint;

mod auth;
mod docling_settings;
mod documents;
mod namespaces;
mod provider_accounts;
mod rows;
mod runtime_settings;
mod search_cache;
mod search_settings;
mod source_connections;
mod sync_runs;

pub use auth::RefreshTokenRecord;
use rows::*;

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
    pub provider_account_key: Option<String>,
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
pub struct StoredProviderAccount {
    pub account_key: String,
    pub provider_kind: String,
    pub display_name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub disabled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeQdrantSettings {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeEmbeddingSettings {
    pub provider_account_key: String,
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
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .context("failed to connect app_db pool")?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("failed to run app_db migrations")?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}
