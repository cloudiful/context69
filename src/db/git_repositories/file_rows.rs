//! Database row shapes for generation-scoped code content (issue #681 work unit
//! 3B2).
//!
//! Rows are private to the module: every stored struct is built from one of
//! these, so a query can never hand a half-populated record to a caller. The
//! aggregate counters the two replace statements return are rows too, because
//! they report what a write actually did.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// One stored manifest entry.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitGenerationFileRow {
    pub file_key: Uuid,
    pub generation_key: Uuid,
    pub repository_key: Uuid,
    pub provider_blob_sha: String,
    pub path: String,
    pub language: String,
    pub byte_count: i64,
    pub line_count: i64,
    pub created_at: DateTime<Utc>,
}

/// One stored chunk with its inclusive line range.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitGenerationChunkRow {
    pub chunk_key: Uuid,
    pub generation_key: Uuid,
    pub file_key: Uuid,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub chunk_text: String,
    pub created_at: DateTime<Utc>,
}

/// What one manifest replacement did.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitManifestReplaceRow {
    pub scoped_generation_count: i64,
    pub conflicting_blob_count: i64,
    pub duplicate_path_count: i64,
    pub stored_blob_count: i64,
    pub retired_file_count: i64,
    pub retired_blob_count: i64,
    pub stored_file_count: i64,
}

/// What one chunk replacement did.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitChunkReplaceRow {
    pub scoped_file_count: i64,
    pub retired_chunk_count: i64,
    pub stored_chunk_count: i64,
}

/// One bounded lexical hit with its repository, ref, commit, path, and line
/// provenance.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitLexicalChunkHitRow {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    pub ref_name: String,
    pub commit_sha: String,
    pub group_visibility: String,
    pub file_key: Uuid,
    pub path: String,
    pub language: String,
    pub chunk_key: Uuid,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub chunk_text: String,
    pub score: f32,
    pub matched: String,
}
