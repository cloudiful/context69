//! MCP-only code-search and code-window inputs and bounded results.
//!
//! `search_code` is a distinct code tool over one Git repository's activated
//! index generation, and `get_code` exposes one bounded, commit-pinned line
//! window of a stored file from that same generation. The stored provenance type
//! [`context69_contracts_sources::sources::GitCodeLexicalHit`] is only an input
//! to the bounded projections below: no blob, connection, or secret material is
//! ever serialized, and every array and free-form string declares the same cap
//! its constructor enforces.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use context69_contracts_core::Visibility;
use context69_contracts_sources::sources::{
    GIT_REPOSITORY_FILE_PATH_MAX_CHARS, GitCodeLexicalHit, GitCodeMatchKind,
    MAX_GIT_CONTENT_WINDOW_LINES,
};

use crate::projections::{
    MCP_CODE_COMMIT_MAX_CHARS, MCP_CODE_LANGUAGE_MAX_CHARS, MCP_CODE_LIMIT_DEFAULT,
    MCP_CODE_LIMIT_MAX, MCP_CODE_LIMIT_MIN, MCP_CODE_PATH_MAX_CHARS,
    MCP_CODE_PATH_PREFIX_MAX_CHARS, MCP_CODE_QUERY_MAX_CHARS, MCP_CODE_REF_MAX_CHARS,
    MCP_CODE_TEXT_MAX_CHARS, MCP_GROUP_PATH_MAX_CHARS, truncate_chars,
};

fn default_mcp_code_limit() -> u8 {
    MCP_CODE_LIMIT_DEFAULT
}

/// MCP-only input for `search_code`.
///
/// The repository is addressed by group path plus its UUID key; the term is
/// matched whole and case-insensitively against stored code, and the optional
/// path-prefix and language filters narrow the same active-generation query.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpCodeSearchRequest {
    #[schemars(length(min = 1, max = 1024))]
    pub group_path: String,
    pub repository_key: Uuid,
    #[schemars(length(min = 1, max = 200))]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 512))]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 32))]
    pub language: Option<String>,
    #[serde(default = "default_mcp_code_limit")]
    #[schemars(range(min = 1, max = 50))]
    pub limit: u8,
}

impl McpCodeSearchRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.group_path.trim().is_empty()
            || self.group_path.chars().count() > MCP_GROUP_PATH_MAX_CHARS
        {
            return Err(anyhow::anyhow!("group_path must be 1..=1024 characters"));
        }
        if self.query.trim().is_empty() {
            return Err(anyhow::anyhow!("query must not be blank"));
        }
        if self.query.chars().count() > MCP_CODE_QUERY_MAX_CHARS {
            return Err(anyhow::anyhow!("query must be at most 200 characters"));
        }
        if self.limit < MCP_CODE_LIMIT_MIN || self.limit > MCP_CODE_LIMIT_MAX {
            return Err(anyhow::anyhow!("limit must be between 1 and 50"));
        }
        if let Some(prefix) = self.path_prefix.as_deref() {
            if prefix.trim().is_empty() || prefix.chars().count() > MCP_CODE_PATH_PREFIX_MAX_CHARS {
                return Err(anyhow::anyhow!("path_prefix must be 1..=512 characters"));
            }
            if prefix.chars().any(char::is_control) {
                return Err(anyhow::anyhow!(
                    "path_prefix must not contain control characters"
                ));
            }
            if prefix.starts_with('/') || prefix.split('/').any(|segment| segment == "..") {
                return Err(anyhow::anyhow!(
                    "path_prefix must be a safe repository-relative path prefix"
                ));
            }
        }
        if let Some(language) = self.language.as_deref()
            && !is_language_token(language)
        {
            return Err(anyhow::anyhow!(
                "language must be a lowercase language token of at most 32 characters"
            ));
        }
        Ok(())
    }
}

/// The classified language token shape: `^[a-z0-9][a-z0-9_+#-]{0,31}$`.
fn is_language_token(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    value.chars().count() <= MCP_CODE_LANGUAGE_MAX_CHARS
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '+' | '#' | '-')
        })
}

/// One bounded code hit with the provenance a caller needs to verify it.
///
/// Identifiers, pinned ref/commit, safe path, language, and the inclusive
/// 1-based line range are preserved; the verbatim chunk text is truncated to
/// [`MCP_CODE_TEXT_MAX_CHARS`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpCodeHit {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    #[schemars(length(max = 256))]
    pub ref_name: String,
    #[schemars(length(max = 64))]
    pub commit_sha: String,
    pub visibility: Visibility,
    pub file_key: Uuid,
    #[schemars(length(max = 512))]
    pub path: String,
    #[schemars(length(max = 32))]
    pub language: String,
    pub chunk_key: Uuid,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub score: f32,
    pub matched: GitCodeMatchKind,
    #[schemars(length(max = 4000))]
    pub text: String,
}

impl McpCodeHit {
    /// Build a hit from a stored lexical hit, enforcing every schema cap.
    pub fn from_lexical_hit(hit: &GitCodeLexicalHit) -> Self {
        Self {
            repository_key: hit.repository_key,
            generation_key: hit.generation_key,
            generation_number: hit.generation_number,
            ref_name: truncate_chars(&hit.ref_name, MCP_CODE_REF_MAX_CHARS),
            commit_sha: truncate_chars(&hit.commit_sha, MCP_CODE_COMMIT_MAX_CHARS),
            visibility: hit.visibility,
            file_key: hit.file_key,
            path: truncate_chars(&hit.path, MCP_CODE_PATH_MAX_CHARS),
            language: truncate_chars(&hit.language, MCP_CODE_LANGUAGE_MAX_CHARS),
            chunk_key: hit.chunk_key,
            chunk_index: hit.chunk_index,
            start_line: hit.start_line,
            end_line: hit.end_line,
            score: hit.score,
            matched: hit.matched,
            text: truncate_chars(&hit.text, MCP_CODE_TEXT_MAX_CHARS),
        }
    }
}

/// Coverage counters of the active generation a code search answered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct McpCodeCoverage {
    pub file_count: i64,
    pub excluded_file_count: i64,
    pub total_bytes: i64,
}

/// Active-generation provenance: repository, pinned ref/commit, freshness, and
/// coverage. Only bounded, non-secret values cross this boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpCodeGeneration {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    #[schemars(length(max = 256))]
    pub ref_name: String,
    #[schemars(length(max = 64))]
    pub commit_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub coverage: McpCodeCoverage,
}

impl McpCodeGeneration {
    pub fn new(
        repository_key: Uuid,
        generation_key: Uuid,
        generation_number: i64,
        ref_name: &str,
        commit_sha: &str,
        completed_at: Option<DateTime<Utc>>,
        coverage: McpCodeCoverage,
    ) -> Self {
        Self {
            repository_key,
            generation_key,
            generation_number,
            ref_name: truncate_chars(ref_name, MCP_CODE_REF_MAX_CHARS),
            commit_sha: truncate_chars(commit_sha, MCP_CODE_COMMIT_MAX_CHARS),
            completed_at,
            coverage,
        }
    }
}

/// Bounded `search_code` output: active-generation provenance, at most
/// [`MCP_CODE_LIMIT_MAX`] hits, and a truthful `truncated` flag instead of a
/// cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpCodeSearchResponse {
    pub generation: McpCodeGeneration,
    #[schemars(length(max = 50))]
    pub hits: Vec<McpCodeHit>,
    pub truncated: bool,
}

impl McpCodeSearchResponse {
    /// Assemble a bounded response. `hits` must have been fetched with
    /// `limit + 1`; at most `limit` are kept and `truncated` reports the drop.
    pub fn new(generation: McpCodeGeneration, mut hits: Vec<McpCodeHit>, limit: usize) -> Self {
        let truncated = hits.len() > limit;
        hits.truncate(limit);
        Self {
            generation,
            hits,
            truncated,
        }
    }
}

/// Default stored-chunk window for `get_code`.
///
/// Mirrors the HTTP content route's chunk-page row bound: one page ends before
/// the chunk that would cross the shared byte cap, so this row bound keeps a
/// page a single bounded statement result.
pub const MCP_CODE_GET_CHUNK_LIMIT_DEFAULT: u8 = 64;
/// Hard cap for chunks returned by one `get_code` call.
pub const MCP_CODE_GET_CHUNK_LIMIT_MAX: u8 = 64;
/// Minimum stored-chunk window for `get_code`.
pub const MCP_CODE_GET_CHUNK_LIMIT_MIN: u8 = 1;

fn default_mcp_get_code_chunk_limit() -> u8 {
    MCP_CODE_GET_CHUNK_LIMIT_DEFAULT
}

/// MCP-only input for `get_code`.
///
/// One committed file window: group path plus UUID repository key, a safe
/// repository-relative path, an inclusive 1-based line window, and an optional
/// bounded stored-chunk limit. The window is served only from the repository's
/// active, ready index generation, and the path is validated again against the
/// acquisition tree-path rules before any storage lookup.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpGetCodeRequest {
    #[schemars(length(min = 1, max = 1024))]
    pub group_path: String,
    pub repository_key: Uuid,
    /// Repository-relative path, as stored in the serving generation's manifest.
    #[schemars(length(min = 1, max = 512))]
    pub path: String,
    /// First source line of the window, inclusive and 1-based.
    #[schemars(range(min = 1))]
    pub start_line: i32,
    /// Last source line of the window, inclusive and at least `start_line`.
    #[schemars(range(min = 1))]
    pub end_line: i32,
    /// At most this many stored chunks are read for the window; the byte cap
    /// always applies first. Defaults to [`MCP_CODE_GET_CHUNK_LIMIT_DEFAULT`].
    #[serde(default = "default_mcp_get_code_chunk_limit")]
    #[schemars(range(min = 1, max = 64))]
    pub chunk_limit: u8,
}

impl McpGetCodeRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.group_path.trim().is_empty()
            || self.group_path.chars().count() > MCP_GROUP_PATH_MAX_CHARS
        {
            return Err(anyhow::anyhow!("group_path must be 1..=1024 characters"));
        }
        validate_repository_path(&self.path)?;
        if self.start_line < 1 || self.end_line < 1 {
            return Err(anyhow::anyhow!(
                "start_line and end_line must be positive 1-based lines"
            ));
        }
        if self.end_line < self.start_line {
            return Err(anyhow::anyhow!(
                "end_line must be greater than or equal to start_line"
            ));
        }
        if (self.end_line - self.start_line) as u64 + 1 > MAX_GIT_CONTENT_WINDOW_LINES as u64 {
            return Err(anyhow::anyhow!(
                "the requested line window must be at most {MAX_GIT_CONTENT_WINDOW_LINES} lines"
            ));
        }
        if !(MCP_CODE_GET_CHUNK_LIMIT_MIN..=MCP_CODE_GET_CHUNK_LIMIT_MAX)
            .contains(&self.chunk_limit)
        {
            return Err(anyhow::anyhow!(
                "chunk_limit must be between {MCP_CODE_GET_CHUNK_LIMIT_MIN} and {MCP_CODE_GET_CHUNK_LIMIT_MAX}"
            ));
        }
        Ok(())
    }
}

/// Shape-check one repository-relative path.
///
/// Mirrors the acquisition-side tree-path rules: non-blank, bounded, relative,
/// and free of control bytes and `.`/`..`/empty/backslash segments. The service
/// parses the value into the validated path type again before storage, so this
/// is the MCP boundary's actionable refusal rather than the last line of defense.
fn validate_repository_path(path: &str) -> anyhow::Result<()> {
    if path.trim().is_empty() || path.chars().count() > GIT_REPOSITORY_FILE_PATH_MAX_CHARS {
        return Err(anyhow::anyhow!(
            "path must be 1..={GIT_REPOSITORY_FILE_PATH_MAX_CHARS} characters"
        ));
    }
    if path.chars().any(char::is_control) {
        return Err(anyhow::anyhow!("path must not contain control characters"));
    }
    if path.starts_with('/') {
        return Err(anyhow::anyhow!("path must be repository-relative"));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\') {
            return Err(anyhow::anyhow!(
                "path must be a safe repository-relative path"
            ));
        }
    }
    Ok(())
}

/// The manifest entry one `get_code` window was read from.
///
/// Only the safe manifest projection crosses the boundary: its identifier, the
/// repository-relative path, and the classified language. The provider blob id,
/// raw acquisition content, and storage bookkeeping never do.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpCodeFile {
    pub file_key: Uuid,
    #[schemars(length(max = 512))]
    pub path: String,
    #[schemars(length(max = 32))]
    pub language: String,
}

/// Bounded `get_code` output.
///
/// Carries active-generation provenance, the safe manifest entry, the requested
/// line bounds, the verbatim stored UTF-8 window text, its exact byte count, and
/// a truthful `truncated` flag instead of a cursor. The text cap mirrors the
/// shared HTTP content byte cap.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpGetCodeResponse {
    pub generation: McpCodeGeneration,
    pub file: McpCodeFile,
    /// First requested source line, inclusive.
    pub start_line: i32,
    /// Last requested source line, inclusive.
    pub end_line: i32,
    #[schemars(length(max = 65536))]
    pub text: String,
    /// Exact UTF-8 byte length of `text`.
    pub byte_count: i64,
    /// Whether more window text existed than this response returns.
    pub truncated: bool,
}
