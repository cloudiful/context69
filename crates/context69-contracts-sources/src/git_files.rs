use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
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

/// Maximum characters accepted for a repository-relative path.
///
/// This is the same bound the acquisition path safety validator enforces, stated
/// on the wire so a generated client learns it without reading the server code.
/// A longer value is rejected as a bounded invalid argument, never truncated.
pub const GIT_REPOSITORY_FILE_PATH_MAX_CHARS: usize = 512;

/// Query for one exact repository path.
///
/// The path is a query parameter rather than a path segment because valid
/// repository paths contain `/`; the server parses it with the same path safety
/// validator that admitted the stored entry, so the value that reaches the
/// lookup is exactly the value that was validated at storage time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct GitRepositoryFileQuery {
    /// Repository-relative path, as stored in the serving generation's manifest.
    ///
    /// The utoipa derives need a literal, so they mirror the constant the
    /// runtime validator enforces; both bounds are asserted in the contract
    /// tests.
    #[schema(max_length = 512)]
    #[schemars(length(max = GIT_REPOSITORY_FILE_PATH_MAX_CHARS))]
    #[param(max_length = 512)]
    pub path: String,
}

/// One exact manifest entry of the generation a repository currently serves.
///
/// The entry is the same safe [`GitRepositoryFile`] projection the manifest page
/// returns, so a caller can compare a detail read against the page it came from
/// field for field. The serving generation's provenance and coverage travel with
/// it, so the entry stays checkable against a pinned commit. File bytes, chunk
/// text, provider blob ids, secret references, and provider transport state
/// never cross this response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryFileDetailResponse {
    pub repository_key: Uuid,
    /// Generation the entry below belongs to.
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
    /// The exact entry the path named.
    pub file: GitRepositoryFile,
}

/// Longest line window one content read may request.
///
/// The bound is on the requested span, not on what storage holds, so a caller
/// cannot ask for a whole file in one response and every read is answered by a
/// page of stored chunks.
pub const MAX_GIT_CONTENT_WINDOW_LINES: usize = 400;

/// Largest returned UTF-8 text one content read may carry, in bytes.
///
/// Runtime byte accounting is authoritative for the response: a page stops
/// before the chunk that would cross this bound, and a continuation token says
/// so. The database caps a single stored chunk far below this, so a page always
/// makes progress and a continuation always terminates.
pub const MAX_GIT_CONTENT_WINDOW_BYTES: usize = 64 * 1024;

/// Longest accepted continuation token for a content read.
///
/// A continuation is a decimal chunk-row offset this service issued, so the
/// width is bounded: a hostile value can never become a long parse or an
/// unbounded `OFFSET`.
pub const MAX_GIT_CONTENT_CURSOR_MAX_CHARS: usize = 20;

/// Query for one bounded line window of an exact repository path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct GitRepositoryFileContentQuery {
    /// Repository-relative path, as stored in the serving generation's manifest.
    ///
    /// The utoipa derives need a literal, so they mirror the constant the
    /// runtime validator enforces; both bounds are asserted in the contract
    /// tests.
    #[schema(max_length = 512)]
    #[schemars(length(max = GIT_REPOSITORY_FILE_PATH_MAX_CHARS))]
    #[param(max_length = 512)]
    pub path: String,
    /// First source line of the window, inclusive and 1-based.
    #[schema(minimum = 1)]
    #[schemars(range(min = 1))]
    #[param(minimum = 1)]
    pub start_line: i32,
    /// Last source line of the window, inclusive and at least `start_line`.
    #[schema(minimum = 1)]
    #[schemars(range(min = 1))]
    #[param(minimum = 1)]
    pub end_line: i32,
    /// Continuation token from a previous page of the same window. Absent starts
    /// at the first matching chunk.
    ///
    /// The utoipa derive needs a literal, so it mirrors the constant the runtime
    /// validator enforces; the bound is asserted in the contract tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = MAX_GIT_CONTENT_CURSOR_MAX_CHARS))]
    #[param(max_length = 20)]
    pub cursor: Option<String>,
}

/// One bounded page of stored chunk text for an exact line window.
///
/// The text is the stored UTF-8 verbatim — same bytes, same line endings, same
/// trailing whitespace — trimmed to the requested inclusive window, so a caller
/// can quote it or concatenate continuation pages in order. `byte_count` is the
/// exact UTF-8 length of `text`, which is what makes the page checkable, and the
/// generation provenance, the manifest entry, and the requested bounds travel
/// with it so the text is always attributable.
///
/// Raw acquisition blobs, provider blob ids, secret references, and connection
/// state never cross this response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryFileContentResponse {
    pub repository_key: Uuid,
    /// Generation the text and entry belong to.
    pub generation_key: Uuid,
    /// Per-repository monotonic sequence of the serving generation.
    pub generation_number: i64,
    pub ref_name: String,
    /// Pinned snapshot commit the serving generation covers.
    pub commit_sha: String,
    /// Index lifecycle state of the repository source, read at query time.
    pub index_status: GitIndexStatus,
    /// Target/indexed commit checkpoint of the source.
    pub checkpoint: GitCommitCheckpoint,
    /// Manifest entries the serving generation covers.
    pub file_count: i64,
    /// File entries acquisition excluded, so coverage gaps stay visible.
    pub excluded_file_count: i64,
    /// Raw bytes the serving generation covers.
    pub total_bytes: i64,
    /// The manifest entry the path named.
    pub file: GitRepositoryFile,
    /// First requested source line, inclusive.
    pub start_line: i32,
    /// Last requested source line, inclusive.
    pub end_line: i32,
    /// Stored text for the window, verbatim and without normalization.
    pub text: String,
    /// Exact UTF-8 byte length of `text`.
    pub byte_count: i64,
    /// Cursor continuation: `has_more = true` always carries `next_cursor`.
    pub pagination: CursorPagination,
}

/// Fewest hits one search may ask for.
pub const GIT_CODE_SEARCH_LIMIT_MIN: u8 = 1;
/// Most hits one search may ask for.
pub const GIT_CODE_SEARCH_LIMIT_MAX: u8 = 50;
/// Hits a search asks for when the caller names no limit.
pub const GIT_CODE_SEARCH_LIMIT_DEFAULT: u8 = 20;
/// Longest accepted search term, in characters.
pub const GIT_CODE_SEARCH_QUERY_MAX_CHARS: usize = 200;
/// Longest accepted repository-relative path prefix, in characters.
pub const GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS: usize = 512;
/// Longest accepted language token, in characters.
pub const GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS: usize = 32;

fn default_code_search_limit() -> u8 {
    GIT_CODE_SEARCH_LIMIT_DEFAULT
}

/// Query for a bounded lexical search over one repository's active generation.
///
/// The term is matched whole and case-insensitively against stored code and is
/// never rewritten here, so a `%` or `_` stays a literal character.
/// `path_prefix` narrows to a repository-relative prefix and is matched
/// case-sensitively, the way Git paths are; `language` is the classified token
/// the manifest stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct GitCodeSearchQuery {
    /// Search term, matched whole and case-insensitively.
    #[schemars(length(min = 1, max = GIT_CODE_SEARCH_QUERY_MAX_CHARS))]
    #[param(min_length = 1, max_length = 200)]
    pub query: String,
    /// Optional repository-relative path prefix, matched case-sensitively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS))]
    #[param(max_length = 512)]
    pub path_prefix: Option<String>,
    /// Optional classified language token, such as `rust` or `markdown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS))]
    #[param(max_length = 32)]
    pub language: Option<String>,
    /// Most hits to return, 1..=50.
    #[serde(default = "default_code_search_limit")]
    #[schemars(range(min = 1, max = 50))]
    #[param(minimum = 1, maximum = 50)]
    pub limit: u8,
}

/// One bounded code hit with the provenance a caller needs to check it.
///
/// The stored text is verbatim — same bytes, same line endings, same trailing
/// whitespace — so a caller can quote it or cite its inclusive line range. Raw
/// acquisition blobs, provider blob ids, secret references, and connection state
/// never cross this contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitCodeSearchHit {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    /// Per-repository monotonic sequence of the serving generation.
    pub generation_number: i64,
    pub ref_name: String,
    /// Pinned snapshot commit the serving generation covers.
    pub commit_sha: String,
    /// Current visibility of the owning group, read at query time.
    pub visibility: Visibility,
    pub file_key: Uuid,
    /// Repository-relative path the hit was found in.
    pub path: String,
    /// Classified language of that path.
    pub language: String,
    pub chunk_key: Uuid,
    /// Zero-based position of the chunk inside its file.
    pub chunk_index: i32,
    /// First source line of the chunk, inclusive and 1-based.
    pub start_line: i32,
    /// Last source line of the chunk, inclusive.
    pub end_line: i32,
    pub text: String,
    pub score: f32,
    /// Which side of the match produced this hit.
    pub matched: GitCodeMatchKind,
}

/// One bounded page of lexical hits over a repository's active generation.
///
/// The response names the serving generation, so every hit stays attributable to
/// a pinned commit, and it reports coverage next to the hits, so a caller can see
/// what the search did not cover. `truncated` is true only when the storage layer
/// held more hits than the page returns. No cursor is offered: a caller that
/// needs more narrows its filters instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitCodeSearchResponse {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    /// Per-repository monotonic sequence of the serving generation.
    pub generation_number: i64,
    pub ref_name: String,
    /// Pinned snapshot commit the serving generation covers.
    pub commit_sha: String,
    /// Index lifecycle state of the source, read at query time.
    pub index_status: GitIndexStatus,
    /// Target/indexed commit checkpoint of the source.
    pub checkpoint: GitCommitCheckpoint,
    /// Manifest entries the serving generation covers.
    pub file_count: i64,
    /// File entries acquisition excluded, so coverage gaps stay visible.
    pub excluded_file_count: i64,
    pub total_bytes: i64,
    /// At most `limit` hits, in the storage layer's score/path/chunk order.
    pub hits: Vec<GitCodeSearchHit>,
    /// Whether more matching hits existed than this response returns.
    pub truncated: bool,
}
