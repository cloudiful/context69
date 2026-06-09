use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{DocumentChunk, NormalizedDocument};

const DEFAULT_MAX_CHARS: usize = 1200;
const DEFAULT_OVERLAP_CHARS: usize = 200;

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
    let paragraphs = split_paragraphs(&document.body_text);
    let mut chunks = Vec::new();
    let mut current = String::new();

    for paragraph in paragraphs {
        let candidate = if current.is_empty() {
            paragraph.clone()
        } else {
            format!("{current}\n\n{paragraph}")
        };

        if candidate.chars().count() <= config.max_chars {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            chunks.push(current);
            current = build_overlap(&chunks, config.overlap_chars);
        }

        if current.is_empty() {
            current = paragraph.clone();
        } else {
            current = format!("{current}\n\n{paragraph}");
        }

        while current.chars().count() > config.max_chars {
            let head = take_chars(&current, config.max_chars);
            chunks.push(head.clone());
            current = build_overlap_from_text(&head, config.overlap_chars);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| DocumentChunk {
            id: chunk_uuid(
                source_key,
                &document.external_id,
                &document.record_hash,
                index as i32,
            ),
            document_id,
            chunk_index: index as i32,
            text,
            record_hash: document.record_hash.clone(),
        })
        .collect()
}

fn split_paragraphs(body: &str) -> Vec<String> {
    body.split("\n\n")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn build_overlap(existing_chunks: &[String], overlap_chars: usize) -> String {
    existing_chunks
        .last()
        .map(|last| build_overlap_from_text(last, overlap_chars))
        .unwrap_or_default()
}

fn build_overlap_from_text(text: &str, overlap_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= overlap_chars {
        return text.to_string();
    }
    chars[chars.len() - overlap_chars..].iter().collect()
}

fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn chunk_uuid(source_key: &str, external_id: &str, record_hash: &str, chunk_index: i32) -> Uuid {
    let name = format!("{source_key}:{external_id}:{record_hash}:{chunk_index}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::{ChunkingConfig, chunk_document};
    use crate::domain::NormalizedDocument;

    #[test]
    fn chunks_long_documents() {
        let document = NormalizedDocument {
            external_id: "doc-1".to_string(),
            title: "title".to_string(),
            summary: None,
            body_text: format!("{}\n\n{}", "a".repeat(900), "b".repeat(900)),
            source_uri: "https://example.com".to_string(),
            published_at: None,
            updated_at: Utc::now(),
            metadata_json: json!({}),
            record_hash: "hash".to_string(),
        };

        let chunks = chunk_document(
            1,
            "gov-info",
            &document,
            &ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
        );

        assert!(chunks.len() >= 2);
        assert!(chunks[0].text.len() >= 900);
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.text.contains(&"b".repeat(300)))
        );
    }
}
