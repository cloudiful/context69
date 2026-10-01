//! Exact code text validation and line-aware chunking (issue #681 work unit
//! 3B2).
//!
//! Code is stored as the commit held it. The validator accepts provider bytes
//! only when they decode as UTF-8, carry no NUL, and stay inside the per-file
//! ceiling the storage layer enforces, so the chunker never has to guess what to
//! do with content it cannot store.
//!
//! The chunker cuts on line boundaries and preserves every byte of the text it
//! keeps: a chunk is a verbatim slice of the source, its line range is
//! inclusive and 1-based, and concatenating the chunks of a file reproduces the
//! file exactly.
//!
//! Every emitted chunk stays within the byte bound, which is what makes it
//! storable: a source line longer than the bound (a minified bundle, a
//! one-line lockfile) is cut at UTF-8 character boundaries into pieces that all
//! carry that line's own range, so nothing is dropped, nothing is split mid
//! character, and a citation still names the line the text came from.

use anyhow::{Result, anyhow};

use crate::db::MAX_GIT_FILE_BYTES;
use crate::domain_errors::DomainError;

/// Why provider bytes cannot be stored as code text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeTextRejection {
    /// The bytes are not valid UTF-8, so no exact text can be stored.
    NotUtf8,
    /// The bytes contain a NUL, which no source text carries and which would
    /// truncate every string derived from it.
    ContainsNul,
    /// The file exceeds the per-file storage ceiling.
    TooLarge,
}

impl CodeTextRejection {
    /// Domain error code for a rejected file.
    pub(crate) fn error_code(self) -> &'static str {
        match self {
            Self::NotUtf8 => "git_code_text_not_utf8",
            Self::ContainsNul => "git_code_text_contains_nul",
            Self::TooLarge => "git_file_too_large",
        }
    }
}

/// Provider bytes accepted as exact, storable code text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeText {
    text: String,
    byte_count: i64,
    line_count: i64,
}

impl CodeText {
    /// Validates raw provider bytes as code text.
    pub(crate) fn validate(bytes: &[u8]) -> Result<Self, CodeTextRejection> {
        if bytes.len() > MAX_GIT_FILE_BYTES {
            return Err(CodeTextRejection::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| CodeTextRejection::NotUtf8)?;
        if text.contains('\0') {
            return Err(CodeTextRejection::ContainsNul);
        }
        Ok(Self {
            byte_count: count_as_i64(bytes.len()).ok_or(CodeTextRejection::TooLarge)?,
            line_count: count_as_i64(count_lines(text)).ok_or(CodeTextRejection::TooLarge)?,
            text: text.to_owned(),
        })
    }

    /// The exact stored text.
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    /// Raw byte length, as stored in the manifest.
    pub(crate) fn byte_count(&self) -> i64 {
        self.byte_count
    }

    /// Number of source lines, as stored in the manifest: empty content has no
    /// lines, and a trailing newline ends the last line instead of starting an
    /// empty one.
    pub(crate) fn line_count(&self) -> i64 {
        self.line_count
    }
}

/// Bounds of one chunking pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodeChunkBounds {
    /// Largest emitted chunk, in bytes, and therefore the largest chunk the
    /// storage layer can accept. A source line longer than this is cut at UTF-8
    /// character boundaries; a character longer than this is never split, so
    /// keep the bound at least four bytes.
    pub max_bytes: usize,
    /// Largest chunk, in source lines.
    pub max_lines: usize,
    /// Largest number of chunks one file may produce.
    pub max_chunks: usize,
}

impl Default for CodeChunkBounds {
    fn default() -> Self {
        Self {
            max_bytes: 4 * 1024,
            max_lines: 200,
            max_chunks: 4_096,
        }
    }
}

/// One verbatim chunk with the inclusive 1-based line range it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeChunk {
    /// Zero-based position inside the file.
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub text: String,
}

/// Cuts `text` into verbatim, line-anchored chunks under the given bounds.
///
/// Every emitted chunk is at most `bounds.max_bytes`, which is the guarantee the
/// stored chunk ceiling needs: a line that cannot fit a chunk on its own is cut
/// at UTF-8 character boundaries into pieces that all carry that line's number,
/// so the pieces reassemble into the line byte for byte and a citation still
/// names the line they came from.
pub(crate) fn chunk_code_text(text: &str, bounds: CodeChunkBounds) -> Result<Vec<CodeChunk>> {
    let mut chunks: Vec<CodeChunk> = Vec::new();
    let mut pending = String::new();
    let mut pending_start_line = 1_i32;
    let mut next_line = 1_i32;

    for line in text.split_inclusive('\n') {
        let line_number = next_line;
        next_line += 1;
        if line.len() > bounds.max_bytes {
            // A line this long never joins another chunk, so whatever is pending
            // is closed first and the line is cut on its own.
            if !pending.is_empty() {
                push_chunk(
                    &mut chunks,
                    bounds,
                    pending_start_line,
                    line_number - 1,
                    std::mem::take(&mut pending),
                )?;
            }
            let mut piece_start = 0;
            while piece_start < line.len() {
                let piece_end = char_boundary(line, piece_start, bounds.max_bytes);
                push_chunk(
                    &mut chunks,
                    bounds,
                    line_number,
                    line_number,
                    line[piece_start..piece_end].to_owned(),
                )?;
                piece_start = piece_end;
            }
            pending_start_line = next_line;
            continue;
        }
        let pending_lines = (line_number - pending_start_line) as usize;
        let closes_pending = !pending.is_empty()
            && (pending.len() + line.len() > bounds.max_bytes || pending_lines >= bounds.max_lines);
        if closes_pending {
            push_chunk(
                &mut chunks,
                bounds,
                pending_start_line,
                line_number - 1,
                std::mem::take(&mut pending),
            )?;
        }
        if pending.is_empty() {
            pending_start_line = line_number;
        }
        pending.push_str(line);
    }
    if !pending.is_empty() {
        push_chunk(
            &mut chunks,
            bounds,
            pending_start_line,
            next_line - 1,
            pending,
        )?;
    }
    Ok(chunks)
}

/// The end of the longest prefix of `line[start..]` that fits `max_bytes` and ends
/// on a UTF-8 character boundary.
///
/// A character is never split: when `max_bytes` cannot hold the next one, the
/// piece grows past the bound rather than cutting mid character, so the returned
/// index is always past `start`.
fn char_boundary(line: &str, start: usize, max_bytes: usize) -> usize {
    let mut end = (start + max_bytes).min(line.len());
    while end > start && !line.is_char_boundary(end) {
        end -= 1;
    }
    if end == start {
        return line[start..]
            .chars()
            .next()
            .map_or(line.len(), |character| start + character.len_utf8());
    }
    end
}

/// Appends one chunk, refusing a file that would need more rows than the caller
/// bounded. Positions are the chunk count so far, which is zero-based and
/// contiguous.
fn push_chunk(
    chunks: &mut Vec<CodeChunk>,
    bounds: CodeChunkBounds,
    start_line: i32,
    end_line: i32,
    text: String,
) -> Result<()> {
    if chunks.len() >= bounds.max_chunks {
        return Err(anyhow!(DomainError::payload_too_large(
            "git_chunk_count_exceeded"
        )));
    }
    let chunk_index = i32::try_from(chunks.len())
        .map_err(|_| anyhow!(DomainError::payload_too_large("git_chunk_list_too_large")))?;
    chunks.push(CodeChunk {
        chunk_index,
        start_line,
        end_line,
        text,
    });
    Ok(())
}

/// Lines the chunker walks: each line is its terminator plus the text before it,
/// so empty content has no lines.
fn count_lines(text: &str) -> usize {
    text.split_inclusive('\n').count()
}

fn count_as_i64(count: usize) -> Option<i64> {
    i64::try_from(count).ok()
}

#[cfg(test)]
#[path = "code_text_tests.rs"]
mod tests;
