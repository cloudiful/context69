use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use context69_contracts_core::Visibility;
use context69_contracts_core::pagination::CursorPagination;

use super::git_repositories::{GitCommitCheckpoint, GitIndexStatus};

/// Why one chunk was returned by a lexical code query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitCodeMatchKind {
    /// The whole query is the file path.
    PathExact,
    /// The path contains the query.
    PathPhrase,
    /// The chunk text contains the query.
    ChunkPhrase,
    /// The chunk text contains every token of the query.
    ChunkTerms,
}

impl GitCodeMatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PathExact => "path_exact",
            Self::PathPhrase => "path_phrase",
            Self::ChunkPhrase => "chunk_phrase",
            Self::ChunkTerms => "chunk_terms",
        }
    }
}

/// One manifest entry: a safe repository path mapped to a stored blob.
///
/// The manifest is scoped to a single index generation, so the same path can
/// hold different content in two snapshots and a superseded snapshot keeps
/// serving its own bytes. `line_count` counts the source lines the chunker
/// walked, so an empty file is `0` and a file ending in a newline does not gain
/// a phantom last line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryFile {
    pub file_key: Uuid,
    pub generation_key: Uuid,
    pub repository_key: Uuid,
    /// Repository-relative path, already checked for traversal, absolute, and
    /// control-character hazards before storage.
    pub path: String,
    /// Classified language token (`rust`, `python`, `unknown`, …) derived from
    /// the path alone.
    pub language: String,
    /// Raw byte length of the stored blob.
    pub byte_count: i64,
    pub line_count: i64,
    pub created_at: DateTime<Utc>,
}

/// One exact chunk of a stored file, anchored to inclusive 1-based lines.
///
/// `text` is a verbatim slice of the stored bytes: whitespace, line endings, and
/// punctuation are preserved, so a caller can quote it or re-render it without
/// having re-normalised anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitCodeChunk {
    pub chunk_key: Uuid,
    pub file_key: Uuid,
    pub generation_key: Uuid,
    /// Zero-based position of the chunk inside its file.
    pub chunk_index: i32,
    /// First source line of the chunk, inclusive and 1-based.
    pub start_line: i32,
    /// Last source line of the chunk, inclusive and 1-based.
    pub end_line: i32,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// One bounded lexical hit against the active generation of a repository.
///
/// Every hit carries the provenance a code result needs to be checkable:
/// repository, generation, pinned ref and commit, path, and the inclusive line
/// range the text came from. `visibility` is the owning group's current
/// visibility, read at query time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitCodeLexicalHit {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    /// Per-repository monotonic sequence of the serving generation.
    pub generation_number: i64,
    pub ref_name: String,
    /// Pinned snapshot commit the serving generation covers.
    pub commit_sha: String,
    pub visibility: Visibility,
    pub file_key: Uuid,
    pub path: String,
    pub language: String,
    pub chunk_key: Uuid,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub text: String,
    pub score: f32,
    pub matched: GitCodeMatchKind,
}

/// One bounded page of the active generation's path manifest.
///
/// The page answers from the generation the repository currently serves, so
/// every entry belongs to the same pinned commit and `generation_number`
/// reported here. The source's own index status and target/indexed checkpoint
/// travel with the page so freshness and coverage gaps stay visible without a
/// second request. File bytes, chunk text, secret references, and provider
/// transport state never cross this response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryFileListResponse {
    pub repository_key: Uuid,
    /// Generation the manifest entries below belong to.
    pub generation_key: Uuid,
    /// Per-repository monotonic sequence of the serving generation.
    pub generation_number: i64,
    pub ref_name: String,
    /// Pinned snapshot commit the serving generation covers.
    pub commit_sha: String,
    /// Index lifecycle state of the repository source, read at query time.
    pub index_status: GitIndexStatus,
    /// Target/indexed commit checkpoint of the source, so the caller can tell a
    /// fresh generation from one the ref has already moved past.
    pub checkpoint: GitCommitCheckpoint,
    /// Manifest entries the serving generation covers.
    pub file_count: i64,
    /// File entries acquisition excluded, so coverage gaps stay visible.
    pub excluded_file_count: i64,
    /// Raw bytes the serving generation covers.
    pub total_bytes: i64,
    /// One page of the manifest, ordered by path.
    pub files: Vec<GitRepositoryFile>,
    /// Cursor continuation: `has_more = true` always carries `next_cursor`.
    pub pagination: CursorPagination,
}
