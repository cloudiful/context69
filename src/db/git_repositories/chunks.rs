//! Group-scoped chunk persistence and bounded lexical code search (issue #681
//! work unit 3B2).
//!
//! Chunk text is stored verbatim with the inclusive line range it was cut from,
//! and one replace call rewrites a file's whole chunk list, so a file is never
//! readable with a mix of two revisions. The lexical query answers only from a
//! repository's activated generation, reads the owning group's current
//! visibility from `context69.groups`, and bounds its result.
//!
//! Code is matched as code: the caller's text is searched whole, and tokens keep
//! the characters identifiers are made of, so `parse_ref` and
//! `SafeRef::parse` are never shredded into prose search terms.

use anyhow::Result;
use uuid::Uuid;

use crate::contracts::sources::GitCodeLexicalHit;

use super::file_rows::{GitChunkReplaceRow, GitGenerationChunkRow, GitLexicalChunkHitRow};
use super::file_types::{
    GitChunkLineWindow, GitChunkReplacement, GitGenerationChunkList, GitLexicalCodeSearch,
    MAX_GIT_CHUNKS_PER_FILE, MAX_GIT_LEXICAL_LIMIT, MAX_GIT_SEARCH_TERM_LENGTH,
    NewGitGenerationChunk, StoredGitGenerationChunk, StoredGitGenerationFile, lexical_hit_from_row,
};
use super::files::bounded_page;
use crate::db::Database;
use crate::domain_errors::DomainError;

impl Database {
    /// Replaces the whole chunk list of one file of a building generation, in a
    /// single statement.
    ///
    /// Returns an error, writing nothing, when the file is unknown, belongs to
    /// another group or generation, or its generation is no longer building.
    pub async fn replace_git_file_chunks(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        file_key: Uuid,
        chunks: &[NewGitGenerationChunk],
    ) -> Result<GitChunkReplacement> {
        validate_chunks(chunks)?;
        let list = GitGenerationChunkList::build(chunks);
        let row = sqlx::query_file_as!(
            GitChunkReplaceRow,
            "src/sql/db/git_repository_files/replace_git_file_chunks.sql",
            group_id,
            repository_key,
            generation_key,
            file_key,
            &list.chunk_indexes,
            &list.start_lines,
            &list.end_lines,
            &list.texts
        )
        .fetch_one(&self.pool)
        .await?;
        GitChunkReplacement::from_row(row)
    }

    /// Lists one bounded page of a file's chunks, in source order.
    pub async fn list_git_generation_chunks(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        file_key: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredGitGenerationChunk>> {
        bounded_page(limit, offset)?;
        let rows = sqlx::query_file_as!(
            GitGenerationChunkRow,
            "src/sql/db/git_repository_files/list_git_generation_chunks.sql",
            group_id,
            repository_key,
            generation_key,
            file_key,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(StoredGitGenerationChunk::from_row)
            .collect())
    }

    /// Lists one bounded page of the stored chunk text covering a line window.
    ///
    /// The page is confined to one group, repository, generation, and file: the
    /// repository and generation keys are taken from the manifest entry itself,
    /// so the text can only ever come from the entry the caller already resolved
    /// for its own group. Only the chunk table is read — the raw acquisition blob
    /// and its provider blob id are never selected — so a caller cannot reach
    /// bytes outside the stored, line-anchored text.
    pub async fn list_git_generation_chunks_in_line_range(
        &self,
        group_id: i64,
        file: &StoredGitGenerationFile,
        window: &GitChunkLineWindow,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredGitGenerationChunk>> {
        bounded_page(limit, offset)?;
        let rows = sqlx::query_file_as!(
            GitGenerationChunkRow,
            "src/sql/db/git_repository_files/list_git_generation_chunks_in_line_range.sql",
            group_id,
            file.repository_key,
            file.generation_key,
            file.file_key,
            window.start_line,
            window.end_line,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(StoredGitGenerationChunk::from_row)
            .collect())
    }

    /// Bounded lexical search over the code chunks of a repository's active
    /// generation.
    ///
    /// Only an activated, ready generation answers, so every hit describes the
    /// snapshot a reader would be served, and a private repository needs its
    /// owning group in `visible_group_ids`.
    pub async fn lexical_search_git_generation_chunks(
        &self,
        group_id: i64,
        search: &GitLexicalCodeSearch,
    ) -> Result<Vec<GitCodeLexicalHit>> {
        let query = query_terms(&search.query);
        if query.phrase.is_empty() {
            return Ok(Vec::new());
        }
        if query.phrase.chars().count() > MAX_GIT_SEARCH_TERM_LENGTH {
            return Err(DomainError::invalid_argument("git_search_term_too_long").into());
        }
        if !(1..=MAX_GIT_LEXICAL_LIMIT).contains(&search.limit) {
            return Err(DomainError::invalid_argument("git_search_limit_out_of_bounds").into());
        }
        let path_prefix_pattern = search
            .path_prefix
            .as_deref()
            .map(|prefix| format!("{}%", escape_like(prefix)));
        let rows = sqlx::query_file_as!(
            GitLexicalChunkHitRow,
            "src/sql/db/git_repository_files/lexical_search_git_generation_chunks.sql",
            group_id,
            search.repository_key,
            query.phrase,
            query.phrase_pattern,
            &query.tokens,
            path_prefix_pattern.as_deref(),
            search.language.as_deref(),
            &search.visible_group_ids,
            search.limit
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lexical_hit_from_row).collect()
    }
}

/// One query prepared for the lexical statement: the whole phrase, the LIKE
/// pattern built from it, and the code-aware tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitQueryTerms {
    /// Lowercased caller text. The lexical statement uses it only for exact path
    /// equality, never inside a pattern.
    pub phrase: String,
    /// The same phrase as a LIKE pattern, with `\`, `%`, and `_` escaped and the
    /// surrounding wildcards applied. Every pattern the statement builds — the
    /// chunk branch and the path branch alike — uses this one, so a wildcard in
    /// the caller's text stays a literal character instead of widening a match.
    pub phrase_pattern: String,
    /// Escaped identifier-aware tokens; empty when the query is one token.
    pub tokens: Vec<String>,
}

/// Split caller text into the whole phrase and its identifier-aware tokens.
///
/// Only characters that cannot appear inside an identifier split a token, so
/// `parse_ref`, `Self::parse`, and `src/db/mod.rs` each stay one token and no
/// character of the caller's text is dropped. The prose `keyword_terms` splitter
/// is deliberately not reused: it splits on ASCII punctuation, which is exactly
/// what would destroy these tokens.
pub(crate) fn query_terms(query: &str) -> GitQueryTerms {
    let phrase = query.trim().to_lowercase();
    let mut tokens: Vec<String> = Vec::new();
    for token in phrase.split(|character: char| !is_identifier_character(character)) {
        if token.is_empty() {
            continue;
        }
        let escaped = escape_like(token);
        if !tokens.contains(&escaped) {
            tokens.push(escaped);
        }
    }
    GitQueryTerms {
        phrase_pattern: format!("%{}%", escape_like(&phrase)),
        tokens,
        phrase,
    }
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '$' | ':' | '.' | '/' | '-')
}

/// Escape the LIKE metacharacters of caller text so a search for `100%` looks
/// for a percent sign instead of matching every line, and a search for `%`
/// matches only text that contains a percent sign. The statement builds every
/// pattern — chunk text and file path — through this one rule.
fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn validate_chunks(chunks: &[NewGitGenerationChunk]) -> Result<()> {
    if chunks.len() > MAX_GIT_CHUNKS_PER_FILE {
        return Err(DomainError::payload_too_large("git_chunk_list_too_large").into());
    }
    Ok(())
}
