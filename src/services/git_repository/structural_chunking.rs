//! Embedded structural chunking for Rust, TypeScript, and TSX (issue #28
//! phase P3).
//!
//! A pinned tree-sitter grammar parses the exact source text; top-level item
//! boundaries become chunk boundaries so an item keeps its attributes and
//! leading comments. Every emitted chunk remains a verbatim byte slice of the
//! source with an inclusive 1-based line range, and the complete chunk list
//! reassembles the file byte for byte.
//!
//! A segment that exceeds the caller's byte or line bound is split by the exact
//! line chunker, so a giant function or module is still stored under the same
//! limits. Parse errors, unsupported languages, a tree above the boundary
//! budget, and grammars that cannot be loaded all return `None`, and the caller
//! falls back to line chunking rather than dropping content.
//!
//! SQL has no viable pinned grammar for this runtime generation, so SQL is
//! intentionally absent here and uses the explicit statement/text fallback of
//! the line chunker.

use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use tree_sitter::{Language, Node, Parser};

use crate::domain_errors::DomainError;

use super::code_text::{CodeChunk, CodeChunkBounds, chunk_code_text_from_line};

/// Largest number of top-level item boundaries one structural pass accepts.
/// A listing above this falls back to line chunking instead of building a huge
/// boundary table.
const MAX_STRUCTURAL_BOUNDARIES: usize = 4_096;

/// Chunks `text` by top-level syntax items when `language` has a pinned grammar.
///
/// `Ok(None)` means the caller must fall back to line chunking: the language is
/// not structurally supported, the grammar could not be loaded, the parse
/// reported an error, or the tree exceeded the boundary budget.
pub(crate) fn chunk_structural_text(
    language: &str,
    text: &str,
    bounds: CodeChunkBounds,
) -> Result<Option<Vec<CodeChunk>>> {
    if text.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(language) = structural_language(language) else {
        return Ok(None);
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Ok(None);
    }
    let Some(tree) = parser.parse(text, None) else {
        return Ok(None);
    };
    let root = tree.root_node();
    if root.has_error() {
        return Ok(None);
    }

    let boundaries = item_boundaries(root, text);
    if boundaries.len() > MAX_STRUCTURAL_BOUNDARIES {
        return Ok(None);
    }

    let mut chunks = Vec::new();
    for (position, start) in boundaries.iter().enumerate() {
        let end = boundaries.get(position + 1).copied().unwrap_or(text.len());
        let segment = &text[*start..end];
        let first_line = line_for_byte(text, *start);
        push_segment(&mut chunks, segment, first_line, bounds)?;
    }

    if chunks.len() > bounds.max_chunks {
        return Err(anyhow!(DomainError::payload_too_large(
            "git_chunk_count_exceeded"
        )));
    }
    for (position, chunk) in chunks.iter_mut().enumerate() {
        chunk.chunk_index = i32::try_from(position)
            .map_err(|_| anyhow!(DomainError::payload_too_large("git_chunk_list_too_large")))?;
    }
    Ok(Some(chunks))
}

fn structural_language(language: &str) -> Option<Language> {
    static RUST: OnceLock<Language> = OnceLock::new();
    static TYPESCRIPT: OnceLock<Language> = OnceLock::new();
    static TSX: OnceLock<Language> = OnceLock::new();
    match language {
        "rust" => Some(
            RUST.get_or_init(|| tree_sitter_rust::LANGUAGE.into())
                .clone(),
        ),
        "typescript" => Some(
            TYPESCRIPT
                .get_or_init(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .clone(),
        ),
        "tsx" => Some(
            TSX.get_or_init(|| tree_sitter_typescript::LANGUAGE_TSX.into())
                .clone(),
        ),
        _ => None,
    }
}

/// Byte offsets where top-level chunks start.
///
/// The first offset is always zero. Each later offset starts the trivia run
/// (comments, attributes, decorators) that belongs to the following item, so a
/// comment or attribute stays with its declaration. The first item never adds a
/// boundary: any leading license header or blank line belongs to it.
fn item_boundaries(root: Node<'_>, text: &str) -> Vec<usize> {
    let mut boundaries = vec![0usize];
    let mut cursor = root.walk();
    let mut pending_trivia_start: Option<usize> = None;
    let mut saw_item = false;
    for child in root.named_children(&mut cursor) {
        if is_trivia(child.kind()) {
            pending_trivia_start.get_or_insert(child.start_byte());
            continue;
        }
        let start = pending_trivia_start.take().unwrap_or(child.start_byte());
        if saw_item {
            let line_start = line_start_for_byte(text, start);
            if boundaries.last().copied() != Some(line_start) {
                boundaries.push(line_start);
            }
        } else {
            saw_item = true;
        }
    }
    boundaries
}

fn is_trivia(kind: &str) -> bool {
    matches!(
        kind,
        "line_comment"
            | "block_comment"
            | "comment"
            | "attribute_item"
            | "inner_attribute_item"
            | "decorator"
            | "hash_bang_line"
            | "shebang"
    )
}

fn push_segment(
    chunks: &mut Vec<CodeChunk>,
    segment: &str,
    first_line: i32,
    bounds: CodeChunkBounds,
) -> Result<()> {
    let lines = segment.split_inclusive('\n').count();
    if segment.len() <= bounds.max_bytes && lines <= bounds.max_lines {
        chunks.push(CodeChunk {
            chunk_index: 0,
            start_line: first_line,
            end_line: first_line + i32::try_from(lines.saturating_sub(1)).unwrap_or(i32::MAX),
            text: segment.to_owned(),
        });
        return Ok(());
    }
    let mut split = chunk_code_text_from_line(segment, bounds, first_line)?;
    chunks.append(&mut split);
    Ok(())
}

/// 1-based line number containing `byte`.
fn line_for_byte(text: &str, byte: usize) -> i32 {
    let prefix = &text[..byte];
    i32::try_from(prefix.bytes().filter(|&value| value == b'\n').count() + 1).unwrap_or(i32::MAX)
}

/// Byte offset of the start of the line containing `byte`.
fn line_start_for_byte(text: &str, byte: usize) -> usize {
    text[..byte].rfind('\n').map_or(0, |newline| newline + 1)
}

#[cfg(test)]
#[path = "structural_chunking_tests.rs"]
mod tests;
