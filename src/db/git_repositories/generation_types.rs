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
};

use super::enums;
use super::rows::{GitActiveGenerationRow, GitRepositoryGenerationRow};

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
