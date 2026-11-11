use std::collections::HashMap;

use qdrant_client::qdrant::{
    CollectionConfig, CollectionInfo, CollectionParams, Distance, FieldType, VectorParams,
    VectorParamsMap, VectorsConfig, vectors_config,
};

use super::super::errors::RebuildTargetCollisionReason;
use super::{
    CODE_PAYLOAD_FIELD_INDEXES, PAYLOAD_FIELD_INDEXES, TargetPreparation,
    is_qdrant_field_index_already_exists, metadata_payload_key, target_preparation,
};

/// Build a `QdrantError` whose `Display` output mirrors the shape the
/// qdrant-client produces for a gRPC `AlreadyExists` response so the
/// idempotency matcher is exercised against real substring patterns
/// without depending on `tonic` directly. `ConversionError` is the only
/// enum variant that takes a free-form `String` we control end-to-end.
fn error_mirroring_response_error(text: &str) -> qdrant_client::QdrantError {
    qdrant_client::QdrantError::ConversionError(format!("Error in the response: {text}"))
}

#[test]
fn code_collection_rejects_a_non_cosine_distance() {
    let dot_collection = collection_info_with_vectors(single_dense_vector(8, Distance::Dot));
    assert_eq!(super::code_vector_size_single(&dot_collection), None);
}

#[test]
fn code_collection_admits_only_a_single_dense_cosine_vector() {
    let cases = [
        (
            "cosine at 8",
            single_dense_vector(8, Distance::Cosine),
            Some(8),
        ),
        (
            "cosine at 1024",
            single_dense_vector(1024, Distance::Cosine),
            Some(1024),
        ),
        ("dot", single_dense_vector(8, Distance::Dot), None),
        ("euclid", single_dense_vector(8, Distance::Euclid), None),
        (
            "manhattan",
            single_dense_vector(8, Distance::Manhattan),
            None,
        ),
        (
            "unknown distance",
            single_dense_vector(8, Distance::UnknownDistance),
            None,
        ),
        (
            "multi-vector cosine",
            vectors_config::Config::Params(VectorParams {
                size: 8,
                distance: Distance::Cosine as i32,
                multivector_config: Some(qdrant_client::qdrant::MultiVectorConfig::default()),
                ..Default::default()
            }),
            None,
        ),
    ];
    for (case, vectors, expected) in cases {
        assert_eq!(
            super::code_vector_size_single(&collection_info_with_vectors(vectors)),
            expected,
            "{case} must be handled as {expected:?}"
        );
    }
}

#[test]
fn code_collection_rejects_named_vector_maps() {
    let named_collection = collection_info_with_vectors(named_dense_vector(8, Distance::Cosine));
    assert_eq!(
        super::code_vector_size_single(&named_collection),
        None,
        "a named-vector map is a different collection type"
    );
}

#[test]
fn document_vector_reader_keeps_its_params_and_params_map_tolerance() {
    assert_eq!(
        super::collection_vector_size(&collection_info_with_vectors(single_dense_vector(
            8,
            Distance::Dot
        ))),
        Some(8),
        "the document path keeps its existing distance tolerance"
    );
    assert_eq!(
        super::collection_vector_size(&collection_info_with_vectors(named_dense_vector(
            8,
            Distance::Euclid
        ))),
        Some(8),
        "the document path keeps its ParamsMap tolerance"
    );
    assert_eq!(
        super::collection_vector_size(&collection_info_with_vectors(
            vectors_config::Config::Params(VectorParams {
                size: 8,
                distance: Distance::Cosine as i32,
                multivector_config: Some(qdrant_client::qdrant::MultiVectorConfig::default()),
                ..Default::default()
            })
        )),
        Some(8),
        "the document path keeps its existing multi-vector tolerance"
    );
}

fn collection_info_with_vectors(vectors: vectors_config::Config) -> CollectionInfo {
    CollectionInfo {
        config: Some(CollectionConfig {
            params: Some(CollectionParams {
                vectors_config: Some(VectorsConfig {
                    config: Some(vectors),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn single_dense_vector(size: u64, distance: Distance) -> vectors_config::Config {
    vectors_config::Config::Params(VectorParams {
        size,
        distance: distance as i32,
        ..Default::default()
    })
}

fn named_dense_vector(size: u64, distance: Distance) -> vectors_config::Config {
    vectors_config::Config::ParamsMap(VectorParamsMap {
        map: HashMap::from([(
            "named".to_string(),
            VectorParams {
                size,
                distance: distance as i32,
                ..Default::default()
            },
        )]),
    })
}

#[test]
fn a_created_target_of_the_matching_dimension_is_only_attached() {
    assert_eq!(
        target_preparation(true, Some(1024), 1024),
        TargetPreparation::Attach,
        "a target this attempt created is attached, never recreated"
    );
}

#[test]
fn a_created_target_of_another_dimension_is_never_adopted() {
    assert_eq!(
        target_preparation(true, Some(768), 1024),
        TargetPreparation::Collision(RebuildTargetCollisionReason::DimensionMismatch),
        "a different vector space is refused, not adopted or emptied"
    );
}

#[test]
fn a_created_target_that_is_gone_is_never_written_under_another_name() {
    assert_eq!(
        target_preparation(true, None, 1024),
        TargetPreparation::Collision(RebuildTargetCollisionReason::Missing),
        "the owned target of this attempt is gone, so the attempt writes nothing"
    );
}

#[test]
fn an_absent_target_without_an_owner_is_created() {
    assert_eq!(
        target_preparation(false, None, 1024),
        TargetPreparation::Create,
        "a target that does not exist yet may be created by this attempt"
    );
}

#[test]
fn an_existing_target_without_an_owner_is_a_collision() {
    assert_eq!(
        target_preparation(false, Some(1024), 1024),
        TargetPreparation::Collision(RebuildTargetCollisionReason::AlreadyExists),
        "an unmarked target that exists is ambiguous and is never adopted"
    );
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

#[test]
fn the_collection_shape_selects_its_own_payload_indexes() {
    assert_eq!(
        super::CollectionShape::Document.payload_field_indexes(),
        PAYLOAD_FIELD_INDEXES,
        "the document shape keeps the baseline payload indexes"
    );
    assert_eq!(
        super::CollectionShape::Code.payload_field_indexes(),
        CODE_PAYLOAD_FIELD_INDEXES,
        "the code shape keeps the isolated code payload indexes"
    );
}

#[test]
fn the_code_collection_indexes_canonical_value_types() {
    let types = CODE_PAYLOAD_FIELD_INDEXES
        .iter()
        .map(|(field, field_type)| (*field, *field_type))
        .collect::<Vec<_>>();
    for keyword in [
        "repository_key",
        "generation_key",
        "file_key",
        "path",
        "language",
    ] {
        assert!(
            types.contains(&(keyword, FieldType::Keyword)),
            "{keyword} must be a keyword index"
        );
    }
    for integer in ["group_id", "chunk_index", "start_line", "end_line"] {
        assert!(
            types.contains(&(integer, FieldType::Integer)),
            "{integer} must be an integer index"
        );
    }
    assert!(
        !CODE_PAYLOAD_FIELD_INDEXES
            .iter()
            .any(|(field, _)| *field == "chunk_key"),
        "the point id carries the chunk key; no redundant filter index is guessed"
    );
}
