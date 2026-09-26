use super::*;
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

fn group() -> PayloadGroup<'static> {
    PayloadGroup {
        group_id: 7,
        group_key: "team",
        group_path: "org/team",
        visibility: Visibility::Public,
    }
}

fn document(metadata: &Value) -> PayloadDocument<'_> {
    PayloadDocument {
        source_key: "file_library",
        external_id: "doc-1",
        title: "Title",
        summary: Some("Summary"),
        source_uri: "context69://doc/1",
        published_at: Some(Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()),
        updated_at_source: Utc.with_ymd_and_hms(2024, 6, 7, 8, 9, 10).unwrap(),
        record_hash: "hash-1",
        metadata_json: metadata,
    }
}

fn chunk_ref(chunk_id: Uuid, document_id: i64, chunk_index: i32, chunk_text: &str) -> ChunkRef<'_> {
    ChunkRef {
        chunk_id,
        document_id,
        chunk_index,
        chunk_text,
    }
}

fn assert_shared_fields(payload: &ChunkPayload, document: &PayloadDocument<'_>) {
    assert_eq!(payload.group_id, 7);
    assert_eq!(payload.group_key, "team");
    assert_eq!(payload.group_path, "org/team");
    assert_eq!(payload.visibility, Visibility::Public);
    assert_eq!(payload.source_key, document.source_key);
    assert_eq!(payload.external_id, document.external_id);
    assert_eq!(payload.title, document.title);
    assert_eq!(payload.summary.as_deref(), document.summary);
    assert_eq!(payload.source_uri, document.source_uri);
    assert_eq!(payload.published_at, document.published_at);
    assert_eq!(payload.updated_at_source, document.updated_at_source);
    assert_eq!(payload.record_hash, document.record_hash);
    assert_eq!(&payload.metadata_json, document.metadata_json);
}

fn assert_original_locale(payload: &ChunkPayload) {
    assert_eq!(payload.content_locale, "original");
    assert_eq!(payload.source_locale, None);
    assert_eq!(payload.translation_provider, None);
}

#[test]
fn seed_uses_placeholder_ids_and_document_body() {
    let metadata = json!({ "source": "unit" });
    let document = document(&metadata);
    let payload = seed(&group(), &document, "full body text");

    assert_eq!(payload.chunk_id, Uuid::nil());
    assert_eq!(payload.document_id, 0);
    assert_eq!(payload.chunk_index, 0);
    assert_eq!(payload.chunk_text, "full body text");
    assert_original_locale(&payload);
    assert_shared_fields(&payload, &document);
}

#[test]
fn business_update_targets_existing_document() {
    let metadata = json!({ "source": "unit" });
    let document = document(&metadata);
    let payload = business_update(&group(), &document, 99, "stored body");

    assert_eq!(payload.chunk_id, Uuid::nil());
    assert_eq!(payload.document_id, 99);
    assert_eq!(payload.chunk_index, 0);
    assert_eq!(payload.chunk_text, "stored body");
    assert_original_locale(&payload);
    assert_shared_fields(&payload, &document);
}

#[test]
fn original_chunk_keeps_chunk_identity() {
    let metadata = json!({ "source": "unit" });
    let document = document(&metadata);
    let chunk_id = Uuid::from_u128(0x1234);
    let payload = original_chunk(
        &group(),
        &document,
        chunk_ref(chunk_id, 42, 3, "chunk body"),
    );

    assert_eq!(payload.chunk_id, chunk_id);
    assert_eq!(payload.document_id, 42);
    assert_eq!(payload.chunk_index, 3);
    assert_eq!(payload.chunk_text, "chunk body");
    assert_original_locale(&payload);
    assert_shared_fields(&payload, &document);
}

#[test]
fn translated_chunk_carries_locale_and_provider() {
    let metadata = json!({ "source": "unit" });
    let document = document(&metadata);
    let chunk_id = Uuid::from_u128(0x5678);
    let payload = translated_chunk(
        &group(),
        &document,
        chunk_ref(chunk_id, 42, 1, "translated body"),
        PayloadLocale {
            content_locale: "fr",
            source_locale: Some("en"),
            translation_provider: Some("deepl"),
        },
    );

    assert_eq!(payload.chunk_id, chunk_id);
    assert_eq!(payload.document_id, 42);
    assert_eq!(payload.chunk_index, 1);
    assert_eq!(payload.chunk_text, "translated body");
    assert_eq!(payload.content_locale, "fr");
    assert_eq!(payload.source_locale.as_deref(), Some("en"));
    assert_eq!(payload.translation_provider.as_deref(), Some("deepl"));
    assert_shared_fields(&payload, &document);
}

#[test]
fn from_row_moves_fields_and_maps_visibility() {
    let payload = from_row(row_payload("public"));

    assert_eq!(payload.chunk_id, Uuid::from_u128(0x9abc));
    assert_eq!(payload.document_id, 11);
    assert_eq!(payload.group_id, 7);
    assert_eq!(payload.group_key, "team");
    assert_eq!(payload.group_path, "org/team");
    assert_eq!(payload.visibility, Visibility::Public);
    assert_eq!(payload.source_key, "file_library");
    assert_eq!(payload.external_id, "doc-1");
    assert_eq!(payload.title, "Title");
    assert_eq!(payload.summary.as_deref(), Some("Summary"));
    assert_eq!(payload.source_uri, "context69://doc/1");
    assert_eq!(payload.record_hash, "hash-1");
    assert_eq!(payload.chunk_index, 5);
    assert_eq!(payload.chunk_text, "chunk body");
    assert_eq!(payload.metadata_json, json!({ "source": "unit" }));
    assert_original_locale(&payload);
}

#[test]
fn from_row_falls_back_to_private_for_unknown_visibility() {
    assert_eq!(
        from_row(row_payload("unexpected")).visibility,
        Visibility::Private
    );
}

fn row_payload(visibility: &str) -> RowPayload {
    RowPayload {
        chunk_id: Uuid::from_u128(0x9abc),
        document_id: 11,
        group_id: 7,
        group_key: "team".to_string(),
        group_path: "org/team".to_string(),
        visibility: visibility.to_string(),
        source_key: "file_library".to_string(),
        external_id: "doc-1".to_string(),
        title: "Title".to_string(),
        summary: Some("Summary".to_string()),
        source_uri: "context69://doc/1".to_string(),
        published_at: Some(Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()),
        updated_at_source: Utc.with_ymd_and_hms(2024, 6, 7, 8, 9, 10).unwrap(),
        record_hash: "hash-1".to_string(),
        chunk_index: 5,
        chunk_text: "chunk body".to_string(),
        metadata_json: json!({ "source": "unit" }),
    }
}
