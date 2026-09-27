use qdrant_client::qdrant::FieldType;

use super::{PAYLOAD_FIELD_INDEXES, is_qdrant_field_index_already_exists, metadata_payload_key};

/// Build a `QdrantError` whose `Display` output mirrors the shape the
/// qdrant-client produces for a gRPC `AlreadyExists` response so the
/// idempotency matcher is exercised against real substring patterns
/// without depending on `tonic` directly. `ConversionError` is the only
/// enum variant that takes a free-form `String` we control end-to-end.
fn error_mirroring_response_error(text: &str) -> qdrant_client::QdrantError {
    qdrant_client::QdrantError::ConversionError(format!("Error in the response: {text}"))
}

#[test]
fn metadata_payload_key_prefixes_namespace() {
    assert_eq!(
        metadata_payload_key("library_file_id"),
        "metadata_index.library_file_id"
    );
}

#[test]
fn payload_index_already_exists_is_treated_as_idempotent_success() {
    for message in [
        "AlreadyExists Field Index already exists {}",
        "AlreadyExists field index already exists on this collection {}",
        "AlreadyExists payload index already exists for field 'library_file_id' {}",
        "AlreadyExists Index already exists for this field name {}",
    ] {
        let error = error_mirroring_response_error(message);
        assert!(
            is_qdrant_field_index_already_exists(&error),
            "expected positive match for {message:?}"
        );
    }
}

#[test]
fn field_index_already_exists_does_not_match_other_codes() {
    // Without the gRPC `AlreadyExists` code, the helper must never match,
    // even if the message text mentions field-index existed.
    for message in [
        "PermissionDenied permission denied for collection {}",
        "InvalidArgument validation failed {}",
        "Unauthenticated transport error: connection refused {}",
        "Internal field index already exists {}",
    ] {
        let error = error_mirroring_response_error(message);
        assert!(
            !is_qdrant_field_index_already_exists(&error),
            "expected negative match for {message:?}"
        );
    }
}

#[test]
fn field_index_already_exists_rejects_unrelated_already_mentions() {
    // Even on the AlreadyExists code, random text containing "already"
    // without the field-index hint must not be classified idempotent.
    for message in [
        "AlreadyExists collection already exists {}",
        "AlreadyExists snapshot already exists {}",
        "AlreadyExists some random unrelated text {}",
    ] {
        let error = error_mirroring_response_error(message);
        assert!(
            !is_qdrant_field_index_already_exists(&error),
            "expected negative match for {message:?}"
        );
    }
}

#[test]
fn payload_field_indexes_preserve_baseline_types() {
    // Exact (field, FieldType) pairs. Collapsing any integer field to
    // Keyword would either fail the Qdrant create_field_index call
    // (wrong schema on integer values) or silently regress filter
    // behaviour; this table pins the baseline types.
    let expected: &[(&str, FieldType)] = &[
        ("group_key", FieldType::Keyword),
        ("group_path", FieldType::Keyword),
        ("group_id", FieldType::Integer),
        ("visibility", FieldType::Keyword),
        ("source_key", FieldType::Keyword),
        ("document_id", FieldType::Integer),
        ("published_ts", FieldType::Integer),
        ("library_file_id", FieldType::Keyword),
    ];
    assert_eq!(PAYLOAD_FIELD_INDEXES, expected);
}

#[test]
fn payload_field_indexes_pin_integer_fields_explicitly() {
    // Dedicated guard for the integer group/document/timestamp fields so
    // a future refactor that accidentally drops them to Keyword fails
    // loudly instead of rebuilding the Qdrant payload schema.
    for field in ["group_id", "document_id", "published_ts"] {
        let entry = PAYLOAD_FIELD_INDEXES
            .iter()
            .find(|(name, _)| *name == field)
            .unwrap_or_else(|| panic!("payload field index missing for {field}"));
        assert_eq!(
            entry.1,
            FieldType::Integer,
            "{field} must remain an Integer payload index"
        );
    }
}

#[test]
fn payload_field_indexes_pin_library_file_id_as_keyword() {
    // The cleanup filter uses `Condition::matches("library_file_id", ...)`,
    // so the field must be indexed as Keyword. A regression to any other
    // type would silently break the fast cleanup path.
    let entry = PAYLOAD_FIELD_INDEXES
        .iter()
        .find(|(name, _)| *name == super::LIBRARY_FILE_ID_FIELD)
        .expect("library_file_id must be present in the payload index table");
    assert_eq!(entry.1, FieldType::Keyword);
}
