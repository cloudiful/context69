//! Index generation types for Git persistence (issue #681 work unit 3B1).
//!
//! Generations are metadata only: the pinned commit a generation covers plus the
//! coverage counters acquisition reported. Content lives in the lexical and
//! vector stores, never in these rows.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::sources::{
    GitActiveGeneration, GitGenerationStatus, GitIndexProfile, GitRepositoryGeneration,
    GitRepositorySkipSummary,
};

use super::enums;
use super::rows::{
    GitActiveGenerationRow, GitGenerationCodeVectorsRow, GitRepositoryGenerationRow,
};

/// A new index generation of a repository, pinned to one commit.
#[derive(Debug, Clone)]
pub struct NewGitRepositoryGeneration {
    pub ref_name: String,
    /// Pinned snapshot commit the generation covers.
    pub commit_sha: String,
    pub index_profile: GitIndexProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitRepositoryGeneration {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    pub ref_name: String,
    pub commit_sha: String,
    pub index_profile: GitIndexProfile,
    pub status: GitGenerationStatus,
    pub file_count: i64,
    /// File entries acquisition excluded, so coverage gaps stay visible.
    pub excluded_file_count: i64,
    pub total_bytes: i64,
    pub error_code: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StoredGitRepositoryGeneration {
    pub(crate) fn from_row(row: GitRepositoryGenerationRow) -> Result<Self> {
        Ok(Self {
            repository_key: row.repository_key,
            generation_key: row.generation_key,
            generation_number: row.generation_number,
            ref_name: row.ref_name,
            commit_sha: row.commit_sha,
            index_profile: enums::index_profile(&row.index_profile)?,
            status: enums::generation_status(&row.status)?,
            file_count: row.file_count,
            excluded_file_count: row.excluded_file_count,
            total_bytes: row.total_bytes,
            error_code: row.error_code,
            started_at: row.started_at,
            completed_at: row.completed_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub fn to_contract(&self) -> GitRepositoryGeneration {
        GitRepositoryGeneration {
            generation_key: self.generation_key,
            repository_key: self.repository_key,
            generation_number: self.generation_number,
            ref_name: self.ref_name.clone(),
            commit_sha: self.commit_sha.clone(),
            index_profile: self.index_profile,
            status: self.status,
            file_count: self.file_count,
            excluded_file_count: self.excluded_file_count,
            total_bytes: self.total_bytes,
            error_code: self.error_code.clone(),
            started_at: self.started_at,
            completed_at: self.completed_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Coverage counters a completed generation records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitGenerationCoverage {
    pub file_count: i64,
    pub excluded_file_count: i64,
    pub total_bytes: i64,
}

/// The checkpoint and fence values one index attempt publishes atomically.
#[derive(Debug, Clone)]
pub struct GitGenerationPublication {
    pub expected_config_revision: i64,
    pub observed_commit_sha: String,
    pub pipeline_version: u32,
    pub config_fingerprint: String,
    pub skip_summary: GitRepositorySkipSummary,
    pub coverage: GitGenerationCoverage,
    pub observed_force_rebuild_requested_at: Option<DateTime<Utc>>,
}

/// The exact code destination and acknowledged point count of one generation.
///
/// `fingerprint` is the destination identity: the code vector-space fingerprint
/// composed with the normalized Qdrant endpoint and dimensions. It never carries
/// the endpoint, a credential, or the document collection name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGitGenerationCodeVectors {
    pub fingerprint: String,
    /// Hash of the normalized Qdrant endpoint that owns these points.
    pub endpoint_identity: String,
    pub dimensions: i64,
    pub collection_name: String,
    pub point_count: i64,
}

/// Code vector metadata as stored for one generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitGenerationCodeVectors {
    pub generation_key: Uuid,
    pub fingerprint: String,
    /// Hash of the normalized endpoint that owns these points. Rows recorded
    /// before this identity existed read `None` and cleanup fails closed.
    pub endpoint_identity: Option<String>,
    pub dimensions: i64,
    pub collection_name: String,
    pub point_count: i64,
    pub created_at: DateTime<Utc>,
}

impl StoredGitGenerationCodeVectors {
    pub(crate) fn from_row(row: GitGenerationCodeVectorsRow) -> Self {
        Self {
            generation_key: row.generation_key,
            fingerprint: row.fingerprint,
            endpoint_identity: row.endpoint_identity,
            dimensions: row.dimensions,
            collection_name: row.collection_name,
            point_count: row.point_count,
            created_at: row.created_at,
        }
    }
}

/// One generation a cleanup pass may remove, with its optional code destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitGenerationCleanupTarget {
    pub generation_key: Uuid,
    pub status: String,
    pub cleanup_pending: bool,
    pub code_vectors: Option<StoredGitGenerationCodeVectors>,
}

impl GitGenerationCleanupTarget {
    pub(crate) fn from_row(
        generation_key: Uuid,
        status: String,
        cleanup_pending: bool,
        code_vectors: Option<StoredGitGenerationCodeVectors>,
    ) -> Self {
        Self {
            generation_key,
            status,
            cleanup_pending,
            code_vectors,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitActiveGeneration {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub activated_at: DateTime<Utc>,
}

impl StoredGitActiveGeneration {
    pub(crate) fn from_row(row: GitActiveGenerationRow) -> Result<Self> {
        Ok(Self {
            repository_key: row.repository_key,
            generation_key: row.generation_key,
            activated_at: row.activated_at,
        })
    }

    pub fn to_contract(&self) -> GitActiveGeneration {
        GitActiveGeneration {
            repository_key: self.repository_key,
            generation_key: self.generation_key,
            activated_at: self.activated_at,
        }
    }
}
