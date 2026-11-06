//! Embedded-enrichment retirement checks.
//!
//! The pure case proves the normalizer behavior the retirement relies on, and
//! the database module ties the durable outbox schema to the worker contract
//! through the reusable PostgreSQL runner.

use chrono::Utc;
use context69::domain::SourceRecord;
use context69::normalize::normalize_record;
use serde_json::{Value, json};

#[path = "embedded_enrichment_migration/database.rs"]
mod database;

fn source(metadata_json: Value) -> SourceRecord {
    SourceRecord {
        external_id: "600519.SH-2026-01-01".to_string(),
        title: "  Company   announcement ".to_string(),
        body_text: "alpha\nbeta".to_string(),
        source_uri: " https://example.test/announcement ".to_string(),
        summary: Some(" short   summary ".to_string()),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json,
    }
}

#[test]
fn stripping_the_embedded_key_preserves_the_current_source_hash() {
    let business = json!({"ts_code": "600519.SH"});
    let merged = json!({
        "ts_code": "600519.SH",
        "extraction": {"stock.news.v1": {"direction": "positive"}}
    });

    // The historical publisher hashed the source before merging enrichment, so
    // the document keeps the business-metadata hash while its stored metadata
    // already carries `extraction`.
    let document_hash = normalize_record(source(business.clone())).record_hash;

    let mut stripped_metadata = merged.clone();
    stripped_metadata
        .as_object_mut()
        .expect("metadata object")
        .remove("extraction");
    let stripped = normalize_record(source(stripped_metadata));
    assert_eq!(stripped.metadata_json, business);
    assert_eq!(
        stripped.record_hash, document_hash,
        "removing extraction and normalizing must reproduce the current hash"
    );

    // Hashing the merged metadata itself does not reproduce the document hash,
    // which is why the migration trusts the immutable canonical snapshot and
    // never recomputes a hash from the merged payload.
    let with_enrichment = normalize_record(source(merged));
    assert_ne!(with_enrichment.record_hash, document_hash);
}
