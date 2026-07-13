use context69_contracts::{
    CreateTextRequest, ImportLibraryFileFromUrlRequest, LibraryTextContentFormat,
    PersonalAccessTokenScope, UpsertLibraryTextRequest,
};
use serde_json::{json, to_value};

#[test]
fn legacy_requests_receive_current_defaults() {
    let text: CreateTextRequest = serde_json::from_value(json!({
        "title": "Plain Doc",
        "content": "Hello"
    }))
    .expect("legacy text request");
    assert_eq!(text.content_format, LibraryTextContentFormat::PlainText);

    let upsert: UpsertLibraryTextRequest = serde_json::from_value(json!({
        "external_id": "doc-1",
        "title": "Doc",
        "content": "Hello",
        "published_at": "2026-06-10T08:30:00+08:00"
    }))
    .expect("legacy upsert request");
    assert_eq!(upsert.metadata_json, json!({}));
    assert_eq!(upsert.content_format, LibraryTextContentFormat::PlainText);
    assert_eq!(
        upsert.published_at.expect("published_at").to_rfc3339(),
        "2026-06-10T00:30:00+00:00"
    );

    let import: ImportLibraryFileFromUrlRequest = serde_json::from_value(json!({
        "url": "https://files.example.test/report.pdf"
    }))
    .expect("legacy URL import request");
    assert!(import.metadata.is_none());
    assert!(import.translation.is_none());
}

#[test]
fn access_token_scope_wire_names_are_stable() {
    assert_eq!(
        to_value([
            PersonalAccessTokenScope::Search,
            PersonalAccessTokenScope::Library,
            PersonalAccessTokenScope::Admin,
        ])
        .expect("serialize scopes"),
        json!(["search", "library", "admin"])
    );
}
