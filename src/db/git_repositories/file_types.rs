//! Stored and insert-side types for generation-scoped code content (issue #681
//! work unit 3B2).
//!
//! Raw bytes live once per (generation, provider blob id); a manifest row points
//! at them through a safe repository path and a classified language, and a chunk
//! row holds verbatim UTF-8 text with the inclusive line range it was cut from.
//! Nothing here rewrites content: no stemming, no case folding of stored text,
//! and no prose term extraction.
//!
//! The byte ceilings here are the Rust side of the storage budget; the migration
//! enforces the same numbers in the database, and the schema tests assert the
//! two stay in step.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::sources::{
    GitCodeChunk, GitCodeLexicalHit, GitCodeMatchKind, GitRepositoryFile,
};
use crate::domain_errors::DomainError;

use super::enums;
use super::file_rows::{
    GitChunkReplaceRow, GitGenerationChunkRow, GitGenerationFileRow, GitLexicalChunkHitRow,
    GitManifestReplaceRow,
};

/// Per-file raw-byte ceiling, mirrored by `chk_git_generation_blobs_size` and
/// `chk_git_generation_files_counts`.
pub const MAX_GIT_FILE_BYTES: usize = 8 * 1024 * 1024;

/// Per-generation ceiling over the deduplicated bytes of one snapshot, mirrored
/// by `context69.git_enforce_generation_byte_budget`.
pub const MAX_GIT_GENERATION_BYTES: i64 = 64 * 1024 * 1024;

/// Largest manifest one replace call may carry, so a single statement stays
/// bounded no matter how large a repository is.
pub const MAX_GIT_MANIFEST_FILES: usize = 16_384;

/// Largest chunk list one replace call may carry.
pub const MAX_GIT_CHUNKS_PER_FILE: usize = 4_096;

/// Largest page a manifest or chunk listing may request.
pub const MAX_GIT_LIST_PAGE: i64 = 1_000;

/// Largest hit count one lexical query may return.
pub const MAX_GIT_LEXICAL_LIMIT: i64 = 200;

/// Longest accepted lexical query, in characters.
pub const MAX_GIT_SEARCH_TERM_LENGTH: usize = 200;

/// One manifest entry to store: a safe repository path, its classified
/// language, the raw bytes, and the line count the chunker walked.
///
/// The byte length is derived from `content` rather than declared, so a caller
/// cannot overstate a file to slip past the per-file ceiling.
#[derive(Debug, Clone)]
pub struct NewGitGenerationFile {
    pub path: String,
    pub language: String,
    pub provider_blob_sha: String,
    pub content: Vec<u8>,
    pub line_count: i64,
}

/// The parallel column lists one manifest replace statement binds.
#[derive(Debug, Clone)]
pub(crate) struct GitGenerationManifest {
    pub paths: Vec<String>,
    pub languages: Vec<String>,
    pub provider_blob_shas: Vec<String>,
    pub contents: Vec<Vec<u8>>,
    pub line_counts: Vec<i64>,
}

impl GitGenerationManifest {
    /// Column lists for `replace_git_generation_files`.
    pub(crate) fn build(files: &[NewGitGenerationFile]) -> Self {
        let mut manifest = Self {
            paths: Vec::with_capacity(files.len()),
            languages: Vec::with_capacity(files.len()),
            provider_blob_shas: Vec::with_capacity(files.len()),
            contents: Vec::with_capacity(files.len()),
            line_counts: Vec::with_capacity(files.len()),
        };
        for file in files {
            manifest.paths.push(file.path.clone());
            manifest.languages.push(file.language.clone());
            manifest
                .provider_blob_shas
                .push(file.provider_blob_sha.clone());
            manifest.contents.push(file.content.clone());
            manifest.line_counts.push(file.line_count);
        }
        manifest
    }
}

/// An inclusive 1-based source line window, validated before the range query
/// runs, so a statement only ever sees a positive, non-reversed, bounded span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitChunkLineWindow {
    pub start_line: i32,
    pub end_line: i32,
}

impl GitChunkLineWindow {
    /// Builds a window, or a bounded invalid-argument error. `max_lines` bounds
    /// the requested span so a caller cannot ask for a whole file.
    pub fn new(start_line: i32, end_line: i32, max_lines: usize) -> Result<Self> {
        if start_line < 1 || end_line < 1 {
            return Err(DomainError::invalid_argument("git_line_window_out_of_bounds").into());
        }
        if end_line < start_line {
            return Err(DomainError::invalid_argument("git_line_window_reversed").into());
        }
        if (end_line - start_line) as u64 + 1 > max_lines as u64 {
            return Err(DomainError::invalid_argument("git_line_window_too_wide").into());
        }
        Ok(Self {
            start_line,
            end_line,
        })
    }
}

/// One chunk to store, with the inclusive 1-based line range its text covers.
#[derive(Debug, Clone)]
pub struct NewGitGenerationChunk {
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub text: String,
}

/// The parallel column lists one chunk replace statement binds.
#[derive(Debug, Clone)]
pub(crate) struct GitGenerationChunkList {
    pub chunk_indexes: Vec<i32>,
    pub start_lines: Vec<i32>,
    pub end_lines: Vec<i32>,
    pub texts: Vec<String>,
}

impl GitGenerationChunkList {
    /// Column lists for `replace_git_file_chunks`.
    pub(crate) fn build(chunks: &[NewGitGenerationChunk]) -> Self {
        let mut list = Self {
            chunk_indexes: Vec::with_capacity(chunks.len()),
            start_lines: Vec::with_capacity(chunks.len()),
            end_lines: Vec::with_capacity(chunks.len()),
            texts: Vec::with_capacity(chunks.len()),
        };
        for chunk in chunks {
            list.chunk_indexes.push(chunk.chunk_index);
            list.start_lines.push(chunk.start_line);
            list.end_lines.push(chunk.end_line);
            list.texts.push(chunk.text.clone());
        }
        list
    }
}

/// A manifest entry as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitGenerationFile {
    pub file_key: Uuid,
    pub generation_key: Uuid,
    pub repository_key: Uuid,
    /// Provider blob id the stored bytes are addressed by. The bytes themselves
    /// are read by the phases that consume them, not through this projection.
    pub provider_blob_sha: String,
    pub path: String,
    pub language: String,
    pub byte_count: i64,
    pub line_count: i64,
    pub created_at: DateTime<Utc>,
}

impl StoredGitGenerationFile {
    pub(crate) fn from_row(row: GitGenerationFileRow) -> Self {
        Self {
            file_key: row.file_key,
            generation_key: row.generation_key,
            repository_key: row.repository_key,
            provider_blob_sha: row.provider_blob_sha,
            path: row.path,
            language: row.language,
            byte_count: row.byte_count,
            line_count: row.line_count,
            created_at: row.created_at,
        }
    }

    /// The validated window this entry may be read over.
    ///
    /// A window reaching past this entry's last line is accepted: the stored text
    /// ends earlier and the caller reads the exact text and byte count returned.
    pub fn line_window(
        &self,
        start_line: i32,
        end_line: i32,
        max_lines: usize,
    ) -> Result<GitChunkLineWindow> {
        GitChunkLineWindow::new(start_line, end_line, max_lines)
    }

    pub fn to_contract(&self) -> GitRepositoryFile {
        GitRepositoryFile {
            file_key: self.file_key,
            generation_key: self.generation_key,
            repository_key: self.repository_key,
            path: self.path.clone(),
            language: self.language.clone(),
            byte_count: self.byte_count,
            line_count: self.line_count,
            created_at: self.created_at,
        }
    }
}

/// A chunk as stored, with its verbatim text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitGenerationChunk {
    pub chunk_key: Uuid,
    pub generation_key: Uuid,
    pub file_key: Uuid,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

impl StoredGitGenerationChunk {
    pub(crate) fn from_row(row: GitGenerationChunkRow) -> Self {
        Self {
            chunk_key: row.chunk_key,
            generation_key: row.generation_key,
            file_key: row.file_key,
            chunk_index: row.chunk_index,
            start_line: row.start_line,
            end_line: row.end_line,
            text: row.chunk_text,
            created_at: row.created_at,
        }
    }

    pub fn to_contract(&self) -> GitCodeChunk {
        GitCodeChunk {
            chunk_key: self.chunk_key,
            file_key: self.file_key,
            generation_key: self.generation_key,
            chunk_index: self.chunk_index,
            start_line: self.start_line,
            end_line: self.end_line,
            text: self.text.clone(),
            created_at: self.created_at,
        }
    }

    /// This chunk's stored text, trimmed to the requested inclusive line window.
    ///
    /// This is the one place a line window is cut, and it uses the same
    /// `split_inclusive('\n')` line rule the chunker used when it stored
    /// `start_line`/`end_line`, so the numbering resolved here is the numbering
    /// that was stored. Bytes are copied verbatim: CRLF endings, trailing
    /// whitespace, and a missing final newline all survive, and a piece of a line
    /// too long to fit one chunk is kept whole because it *is* part of the
    /// requested line rather than a line of its own. A chunk outside the window
    /// yields an empty string, which is how a caller concatenates several chunks
    /// into one window without splitting lines a second way.
    pub fn text_in_line_window(&self, start_line: i32, end_line: i32) -> String {
        if self.end_line < start_line || self.start_line > end_line {
            return String::new();
        }
        let mut trimmed = String::new();
        for (offset, line) in self.text.split_inclusive('\n').enumerate() {
            // A piece cut from the middle of one long line holds a single
            // segment, so its numbering still resolves to that line.
            let line_number = self.start_line.saturating_add(offset as i32);
            if (start_line..=end_line).contains(&line_number) {
                trimmed.push_str(line);
            }
        }
        trimmed
    }
}

/// What one manifest replacement did to a generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitManifestReplacement {
    /// Blobs newly stored; unchanged bytes are deduplicated and not counted.
    pub stored_blob_count: i64,
    /// Manifest rows retired, each of which took its chunks with it.
    pub retired_file_count: i64,
    /// Bytes no described file needs any more.
    pub retired_blob_count: i64,
    pub stored_file_count: i64,
}

impl GitManifestReplacement {
    pub(crate) fn from_row(row: GitManifestReplaceRow) -> Result<Self> {
        if row.scoped_generation_count == 0 {
            return Err(anyhow!(
                "no building git index generation of this group and repository to write"
            ));
        }
        if row.conflicting_blob_count > 0 {
            return Err(anyhow!(
                "git manifest maps one provider blob id to different bytes: {} conflicting id(s)",
                row.conflicting_blob_count
            ));
        }
        if row.duplicate_path_count > 0 {
            return Err(anyhow!(
                "git manifest describes one path more than once: {} duplicate path(s)",
                row.duplicate_path_count
            ));
        }
        Ok(Self {
            stored_blob_count: row.stored_blob_count,
            retired_file_count: row.retired_file_count,
            retired_blob_count: row.retired_blob_count,
            stored_file_count: row.stored_file_count,
        })
    }
}

/// What one chunk replacement did to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitChunkReplacement {
    pub retired_chunk_count: i64,
    pub stored_chunk_count: i64,
}

impl GitChunkReplacement {
    pub(crate) fn from_row(row: GitChunkReplaceRow) -> Result<Self> {
        if row.scoped_file_count == 0 {
            return Err(anyhow!(
                "no building git index generation file of this group to write chunks for"
            ));
        }
        Ok(Self {
            retired_chunk_count: row.retired_chunk_count,
            stored_chunk_count: row.stored_chunk_count,
        })
    }
}

/// One bounded lexical query against a repository's active generation.
#[derive(Debug, Clone)]
pub struct GitLexicalCodeSearch {
    pub repository_key: Uuid,
    /// Caller query text, matched whole and case-insensitively. It is never
    /// stemmed, split on punctuation, or otherwise rewritten.
    pub query: String,
    /// Optional repository-relative path prefix, matched case-sensitively the way
    /// Git paths are.
    pub path_prefix: Option<String>,
    /// Optional classified language token.
    pub language: Option<String>,
    /// Groups the caller may read; a private repository needs its owner here.
    pub visible_group_ids: Vec<i64>,
    pub limit: i64,
}

/// Build one bounded lexical hit from its row, failing closed on a stored value
/// the code does not recognise.
pub(crate) fn lexical_hit_from_row(row: GitLexicalChunkHitRow) -> Result<GitCodeLexicalHit> {
    Ok(GitCodeLexicalHit {
        repository_key: row.repository_key,
        generation_key: row.generation_key,
        generation_number: row.generation_number,
        ref_name: row.ref_name,
        commit_sha: row.commit_sha,
        visibility: enums::visibility(&row.group_visibility)?,
        file_key: row.file_key,
        path: row.path,
        language: row.language,
        chunk_key: row.chunk_key,
        chunk_index: row.chunk_index,
        start_line: row.start_line,
        end_line: row.end_line,
        text: row.chunk_text,
        score: row.score,
        matched: match_kind(&row.matched)?,
    })
}

fn match_kind(value: &str) -> Result<GitCodeMatchKind> {
    match value {
        "path_exact" => Ok(GitCodeMatchKind::PathExact),
        "path_phrase" => Ok(GitCodeMatchKind::PathPhrase),
        "chunk_phrase" => Ok(GitCodeMatchKind::ChunkPhrase),
        "chunk_terms" => Ok(GitCodeMatchKind::ChunkTerms),
        other => Err(anyhow!("unknown git code match kind: {other}")),
    }
}
