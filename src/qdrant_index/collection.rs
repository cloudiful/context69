use anyhow::{Context, Result, anyhow};
use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, FieldType,
        PayloadSchemaType, VectorParamsBuilder,
    },
};
use tracing::warn;

use super::{QdrantIndex, collection_vector_size, format_qdrant_error};
use crate::config::QdrantConfig;

/// Payload fields the collection must be indexed on, with their exact Qdrant
/// `FieldType`. Every entry is created with `create_field_index` during
/// `create_collection`. The baseline types must be preserved exactly:
///
/// - `group_key`, `group_path`, `visibility`, `source_key`, `library_file_id`
///   are `Keyword` (string equality / `matches` filters).
/// - `group_id`, `document_id`, `published_ts` are `Integer` (numeric range
///   filters, joins on the parent row id).
///
/// Collapsing the integer fields to `Keyword` would build the wrong payload
/// schema on new collections and would either fail the `create_field_index`
/// call (Qdrant rejects Keyword index on integer values) or silently force
/// every filter through a less efficient code path. Pre-existing collections
/// already carry these typed indexes; the reconciliation path therefore
/// targets only `library_file_id` (the one field the baseline missed).
const PAYLOAD_FIELD_INDEXES: &[(&str, FieldType)] = &[
    ("group_key", FieldType::Keyword),
    ("group_path", FieldType::Keyword),
    ("group_id", FieldType::Integer),
    ("visibility", FieldType::Keyword),
    ("source_key", FieldType::Keyword),
    ("document_id", FieldType::Integer),
    ("published_ts", FieldType::Integer),
    ("library_file_id", FieldType::Keyword),
];

/// Keyword payload field that drives library file cleanup. Pre-existing
/// collections were created without this index; reconciling it during
/// `connect` keeps the cleanup filter fast and avoids the 30s Qdrant timeout
/// seen in production.
const LIBRARY_FILE_ID_FIELD: &str = "library_file_id";

/// Operation label used when surfacing a non-idempotent `create_field_index`
/// failure through `format_qdrant_error`. Falls through the default prefix
/// branch in `legacy_qdrant_prefix` ("qdrant request failed"); the qdrant
/// substring plus transport / timeout signals that dependency classification
/// looks for still reach `is_qdrant_transient`.
const CREATE_FIELD_INDEX_OPERATION: &str = "create_field_index";

impl QdrantIndex {
    pub async fn ensure_metadata_field_index(&self, path: &str, data_type: &str) -> Result<()> {
        let field_type = match data_type {
            "keyword" => FieldType::Keyword,
            "datetime" => FieldType::Datetime,
            "integer" => FieldType::Integer,
            "boolean" => FieldType::Bool,
            "float" => FieldType::Float,
            value => return Err(anyhow!("unsupported qdrant metadata type {value}")),
        };
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    metadata_payload_key(path),
                    field_type,
                )
                .wait(true),
            )
            .await?;
        Ok(())
    }

    pub async fn delete_metadata_field_index(&self, path: &str) -> Result<()> {
        self.client
            .delete_field_index(
                qdrant_client::qdrant::DeleteFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    metadata_payload_key(path),
                )
                .wait(true),
            )
            .await?;
        Ok(())
    }

    pub async fn connect(config: &QdrantConfig, dimensions: usize) -> Result<(Self, bool)> {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: config.collection_name.clone(),
            dimensions,
        };
        let recreated = index
            .ensure_collection(dimensions, config.recreate_on_dimension_mismatch)
            .await
            .with_context(|| {
                format!(
                    "failed to initialize qdrant collection '{}' at {}; qdrant-client uses the gRPC endpoint, usually port 6334, not the REST endpoint on 6333",
                    config.collection_name, config.url
                )
            })?;
        if !recreated {
            // Pre-existing collection: the original create path missed the
            // `library_file_id` keyword index, and the cleanup filter on that
            // field is what timed out in production. Reconcile here so the
            // index is in place before the first cleanup/delete can run.
            index.ensure_library_file_id_field_index().await?;
        }
        Ok((index, recreated))
    }

    async fn ensure_collection(
        &self,
        dimensions: usize,
        recreate_on_dimension_mismatch: bool,
    ) -> Result<bool> {
        if self.client.collection_exists(&self.collection_name).await? {
            let collection = self
                .client
                .collection_info(&self.collection_name)
                .await?
                .result
                .context("missing qdrant collection info")?;
            let actual_dimensions = collection_vector_size(&collection)
                .context("missing qdrant vector size in collection config")?;
            if actual_dimensions != dimensions {
                if recreate_on_dimension_mismatch {
                    warn!(
                        collection_name = self.collection_name,
                        expected_dimensions = dimensions,
                        actual_dimensions,
                        "qdrant collection dimension mismatch detected, recreating collection"
                    );
                    self.recreate_collection().await?;
                    return Ok(true);
                }
                return Err(anyhow!(
                    "qdrant collection {} dimension mismatch: expected {}, found {}",
                    self.collection_name,
                    dimensions,
                    actual_dimensions
                ));
            }
            return Ok(false);
        }

        self.create_collection(dimensions).await?;
        Ok(true)
    }

    pub async fn recreate_collection(&self) -> Result<()> {
        if self.client.collection_exists(&self.collection_name).await? {
            self.client.delete_collection(&self.collection_name).await?;
        }
        self.create_collection(self.dimensions).await
    }

    async fn create_collection(&self, dimensions: usize) -> Result<()> {
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ),
            )
            .await?;

        for &(field, field_type) in PAYLOAD_FIELD_INDEXES {
            self.ensure_field_index_idempotent(field, field_type)
                .await?;
        }
        Ok(())
    }

    /// Idempotent reconciliation for `library_file_id` on collections that
    /// pre-date the index. Self-checks the live payload schema first so we
    /// avoid an extra round trip when the index is already present, and
    /// swallows only the narrow gRPC `AlreadyExists`/already-indexed signal —
    /// every other failure propagates with the operation / collection /
    /// category formatter used elsewhere in `qdrant_index.rs`.
    pub async fn ensure_library_file_id_field_index(&self) -> Result<()> {
        if self
            .payload_schema_has_keyword_field(LIBRARY_FILE_ID_FIELD)
            .await?
        {
            return Ok(());
        }
        self.ensure_field_index_idempotent(LIBRARY_FILE_ID_FIELD, FieldType::Keyword)
            .await
    }

    async fn payload_schema_has_keyword_field(&self, field: &str) -> Result<bool> {
        let collection = self
            .client
            .collection_info(&self.collection_name)
            .await?
            .result
            .context("missing qdrant collection info")?;
        Ok(collection
            .payload_schema
            .get(field)
            .map(|info| info.data_type() == PayloadSchemaType::Keyword)
            .unwrap_or(false))
    }

    /// Create a payload field index, treating only the narrow
    /// `AlreadyExists`/already-indexed signal as success. Non-idempotent
    /// failures (transport, validation, schema mismatch, etc.) are returned
    /// so callers can route them through the standard Qdrant error formatter.
    async fn ensure_field_index_idempotent(
        &self,
        field: &str,
        field_type: FieldType,
    ) -> Result<()> {
        let request =
            CreateFieldIndexCollectionBuilder::new(self.collection_name.clone(), field, field_type)
                .wait(true);
        match self.client.create_field_index(request).await {
            Ok(_) => Ok(()),
            Err(error) if is_qdrant_field_index_already_exists(&error) => Ok(()),
            Err(error) => Err(format_qdrant_error(
                CREATE_FIELD_INDEX_OPERATION,
                &self.collection_name,
                &format!("field={field} type={field_type:?}"),
                anyhow::Error::new(error),
            )),
        }
    }
}

pub(crate) fn metadata_payload_key(path: &str) -> String {
    format!("metadata_index.{path}")
}

/// Narrow idempotency check: returns true only when Qdrant explicitly reports
/// that this field's payload index already exists (gRPC `AlreadyExists` +
/// field-index-shaped message). Validation / permission / transport errors
/// that happen to mention "already" elsewhere in their text are NOT swallowed.
fn is_qdrant_field_index_already_exists(error: &qdrant_client::QdrantError) -> bool {
    // QdrantError Display includes the gRPC code in PascalCase, e.g.
    // `Error in the response: AlreadyExists Index already exists ...`.
    let text = error.to_string().to_ascii_lowercase();
    let has_already_exists_code = text.contains("alreadyexists");
    if !has_already_exists_code {
        return false;
    }
    let field_index_hint = text.contains("field index")
        || text.contains("payload index")
        || text.contains("index already")
        || text.contains("indexed already");
    let exists_hint = text.contains("already exists")
        || text.contains("already indexed")
        || text.contains("already exist");
    field_index_hint && exists_hint
}

#[cfg(test)]
mod tests {
    use qdrant_client::qdrant::FieldType;

    use super::{
        PAYLOAD_FIELD_INDEXES, is_qdrant_field_index_already_exists, metadata_payload_key,
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
}
