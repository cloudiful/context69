use std::mem;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{DocumentChunk, NormalizedDocument};

const DEFAULT_MAX_CHARS: usize = 1200;
const DEFAULT_OVERLAP_CHARS: usize = 200;
const PARAGRAPH_SEPARATOR: &str = "\n\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
            overlap_chars: DEFAULT_OVERLAP_CHARS,
        }
    }
}

pub fn chunk_document(
    document_id: i64,
    source_key: &str,
    document: &NormalizedDocument,
    config: &ChunkingConfig,
) -> Vec<DocumentChunk> {
    chunk_document_iter(document_id, source_key, document, config).collect()
}

pub fn chunk_document_iter<'a>(
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    config: &ChunkingConfig,
) -> ChunkDocumentIter<'a> {
    ChunkDocumentIter::new(document_id, source_key, document, config)
}

pub struct ChunkDocumentIter<'a> {
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    paragraphs: std::str::Split<'a, &'static str>,
    current: String,
    current_chars: usize,
    pending: Option<PendingParagraph<'a>>,
    max_chars: usize,
    overlap_chars: usize,
    next_index: i32,
    finished: bool,
}

struct PendingParagraph<'a> {
    text: &'a str,
    offset: usize,
    separator_offset: usize,
}

impl<'a> ChunkDocumentIter<'a> {
    fn new(
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        config: &ChunkingConfig,
    ) -> Self {
        let max_chars = config.max_chars.max(1);
        let overlap_chars = config.overlap_chars.min(max_chars - 1);
        let paragraphs = document.body_text.split(PARAGRAPH_SEPARATOR);

        Self {
            document_id,
            source_key,
            document,
            paragraphs,
            current: String::new(),
            current_chars: 0,
            pending: None,
            max_chars,
            overlap_chars,
            next_index: 0,
            finished: false,
        }
    }

    fn next_paragraph(&mut self) -> Option<&'a str> {
        self.paragraphs.find_map(|value| {
            let value = value.trim();
            (!value.is_empty()).then_some(value)
        })
    }

    fn take_chunk(&mut self, keep_overlap: bool) -> String {
        let chunk = mem::take(&mut self.current);
        if keep_overlap {
            self.current = build_overlap_from_text(&chunk, self.overlap_chars);
            self.current_chars = self.current.chars().count();
        } else {
            self.current_chars = 0;
        }
        chunk
    }

    fn document_chunk(&mut self, text: String) -> DocumentChunk {
        let index = self.next_index;
        self.next_index += 1;
        DocumentChunk {
            id: chunk_uuid(
                self.source_key,
                &self.document.external_id,
                &self.document.record_hash,
                index,
            ),
            document_id: self.document_id,
            chunk_index: index,
            text,
            record_hash: self.document.record_hash.clone(),
        }
    }
}

impl Iterator for ChunkDocumentIter<'_> {
    type Item = DocumentChunk;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.finished {
                return None;
            }

            if let Some(mut pending) = self.pending.take() {
                if pending.separator_offset < PARAGRAPH_SEPARATOR.len() {
                    if self.current_chars == self.max_chars {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }
                    let remaining_separator = &PARAGRAPH_SEPARATOR[pending.separator_offset..];
                    let capacity = self.max_chars - self.current_chars;
                    let take_len = prefix_len_by_chars(remaining_separator, capacity);
                    self.current.push_str(&remaining_separator[..take_len]);
                    pending.separator_offset += take_len;
                    self.current_chars += take_len;
                    if pending.separator_offset < PARAGRAPH_SEPARATOR.len() {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }
                }

                if pending.offset < pending.text.len() {
                    if self.current_chars == self.max_chars {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }

                    let remaining = &pending.text[pending.offset..];
                    let capacity = self.max_chars - self.current_chars;
                    let take_len = prefix_len_by_chars(remaining, capacity);
                    self.current.push_str(&remaining[..take_len]);
                    pending.offset += take_len;
                    self.current_chars += remaining[..take_len].chars().count();

                    if pending.offset < pending.text.len() {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }
                }

                continue;
            }

            let Some(paragraph) = self.next_paragraph() else {
                if self.current.is_empty() {
                    self.finished = true;
                    return None;
                }
                let chunk = self.take_chunk(false);
                self.finished = true;
                return Some(self.document_chunk(chunk));
            };

            let paragraph_chars = paragraph.chars().count();
            let separator_chars = if self.current.is_empty() { 0 } else { 2 };
            if self.current_chars + separator_chars + paragraph_chars <= self.max_chars {
                if separator_chars != 0 {
                    self.current.push_str(PARAGRAPH_SEPARATOR);
                }
                self.current.push_str(paragraph);
                self.current_chars += separator_chars + paragraph_chars;
                continue;
            }

            if !self.current.is_empty() {
                let chunk = self.take_chunk(true);
                self.pending = Some(PendingParagraph {
                    text: paragraph,
                    offset: 0,
                    separator_offset: 0,
                });
                return Some(self.document_chunk(chunk));
            }

            self.pending = Some(PendingParagraph {
                text: paragraph,
                offset: 0,
                separator_offset: PARAGRAPH_SEPARATOR.len(),
            });
        }
    }
}

fn build_overlap_from_text(text: &str, overlap_chars: usize) -> String {
    if overlap_chars == 0 {
        return String::new();
    }
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= overlap_chars {
        return text.to_string();
    }
    chars[chars.len() - overlap_chars..].iter().collect()
}

fn prefix_len_by_chars(text: &str, count: usize) -> usize {
    text.char_indices()
        .nth(count)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn chunk_uuid(source_key: &str, external_id: &str, record_hash: &str, chunk_index: i32) -> Uuid {
    let name = format!("{source_key}:{external_id}:{record_hash}:{chunk_index}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

#[cfg(test)]
#[path = "chunking_tests.rs"]
mod tests;
