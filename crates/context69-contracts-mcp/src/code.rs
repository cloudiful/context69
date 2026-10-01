//! MCP-only code-search inputs and bounded results.
//!
//! `search_code` is a distinct code tool over one Git repository's activated
//! index generation. The stored provenance type
//! [`context69_contracts_sources::sources::GitCodeLexicalHit`] is only an input
//! to the bounded projections below: no blob, connection, or secret material is
//! ever serialized, and every array and free-form string declares the same cap
//! its constructor enforces.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use context69_contracts_core::Visibility;
use context69_contracts_sources::sources::{GitCodeLexicalHit, GitCodeMatchKind};

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
