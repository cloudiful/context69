use sha2::{Digest, Sha256};

use crate::domain::{NormalizedDocument, SourceRecord};

pub fn normalize_record(record: SourceRecord) -> NormalizedDocument {
    let title = normalize_whitespace(&record.title);
    let summary = record
        .summary
        .map(|value| normalize_whitespace(&value))
        .filter(|value| is_meaningful_text(value));
    let body_text = normalize_body(&record.body_text);
    let source_uri = record.source_uri.trim().to_string();
    let metadata_json = record.metadata_json;
    let record_hash = hash_document(
        &title,
        summary.as_deref(),
        &body_text,
        &source_uri,
        &metadata_json,
    );

    NormalizedDocument {
        external_id: record.external_id.trim().to_string(),
        title,
        summary,
        body_text,
        source_uri,
        published_at: record.published_at,
        updated_at: record.updated_at,
        metadata_json,
        record_hash,
    }
}

pub fn normalize_body(input: &str) -> String {
    input
        .lines()
        .map(normalize_whitespace)
        .filter(|line| is_meaningful_text(line))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn normalize_whitespace(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub fn is_meaningful_text(input: &str) -> bool {
    input.chars().any(char::is_alphanumeric)
}

fn hash_document(
    title: &str,
    summary: Option<&str>,
    body_text: &str,
    source_uri: &str,
    metadata_json: &serde_json::Value,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(b"\n");
    hasher.update(summary.unwrap_or_default().as_bytes());
    hasher.update(b"\n");
    hasher.update(body_text.as_bytes());
    hasher.update(b"\n");
    hasher.update(source_uri.as_bytes());
    hasher.update(b"\n");
    hasher.update(metadata_json.to_string().as_bytes());
    hasher
        .finalize()
        .as_slice()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::{is_meaningful_text, normalize_record};
    use crate::domain::SourceRecord;

    #[test]
    fn normalizes_whitespace_and_hashes_content() {
        let normalized = normalize_record(SourceRecord {
            external_id: "  doc-1 ".to_string(),
            title: "  Title   Here ".to_string(),
            body_text: "hello   world\n\n  second   line".to_string(),
            source_uri: " https://example.com/doc-1 ".to_string(),
            summary: Some(" a   short   summary ".to_string()),
            published_at: None,
            updated_at: Utc::now(),
            metadata_json: json!({"agency": "x"}),
        });

        assert_eq!(normalized.external_id, "doc-1");
        assert_eq!(normalized.title, "Title Here");
        assert_eq!(normalized.summary.as_deref(), Some("a short summary"));
        assert_eq!(normalized.body_text, "hello world\n\nsecond line");
        assert_eq!(normalized.source_uri, "https://example.com/doc-1");
        assert_eq!(normalized.record_hash.len(), 64);
    }

    #[test]
    fn drops_symbol_only_summary_and_body_lines() {
        let normalized = normalize_record(SourceRecord {
            external_id: "doc-2".to_string(),
            title: "Title".to_string(),
            body_text: ">\n\n---\n\n有效内容\n\n***".to_string(),
            source_uri: "https://example.com/doc-2".to_string(),
            summary: Some(" > ".to_string()),
            published_at: None,
            updated_at: Utc::now(),
            metadata_json: json!({}),
        });

        assert_eq!(normalized.summary, None);
        assert_eq!(normalized.body_text, "有效内容");
    }

    #[test]
    fn meaningful_text_requires_letters_or_numbers() {
        assert!(!is_meaningful_text(">"));
        assert!(!is_meaningful_text("..."));
        assert!(is_meaningful_text("杭州"));
        assert!(is_meaningful_text("policy-1"));
    }
}
