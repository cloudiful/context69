use super::{
    QdrantIndex,
    errors::{RebuildTargetCollision, RebuildTargetCollisionReason},
    format_qdrant_error,
};
use crate::config::QdrantConfig;
use anyhow::{Context, Result, anyhow};
use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, FieldType,
        PayloadSchemaType, VectorParamsBuilder, vectors_config,
    },
};
use std::future::Future;

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

/// The decision one rebuild target preparation makes.
///
/// The decision is pure so the ownership boundary is testable without a
/// Qdrant server: a collection this attempt created is only attached, a
/// collection it did not create is never adopted, and only an absent target
/// may be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetPreparation {
    /// The exact target exists at the expected dimension; attach it.
    Attach,
    /// The exact target is absent; create it at the expected dimension.
    Create,
    /// The target may not be used by this attempt; never attach or write it.
    Collision(RebuildTargetCollisionReason),
}

/// Decides how one rebuild target is prepared.
///
/// `created_by_owner` is the durable ownership marker of the recorded target:
/// it says that the attempt which owns the barrier created that collection
/// itself. A marked target may only be attached, an unmarked target that
/// already exists is ambiguous and is refused, and a target that does not
/// exist may only be created while no other attempt could have created it.
pub(crate) fn target_preparation(
    created_by_owner: bool,
    existing_dimensions: Option<usize>,
    expected_dimensions: usize,
) -> TargetPreparation {
    match existing_dimensions {
        Some(dimensions) if dimensions != expected_dimensions => {
            TargetPreparation::Collision(RebuildTargetCollisionReason::DimensionMismatch)
        }
        Some(_) if created_by_owner => TargetPreparation::Attach,
        Some(_) => TargetPreparation::Collision(RebuildTargetCollisionReason::AlreadyExists),
        None if created_by_owner => {
            TargetPreparation::Collision(RebuildTargetCollisionReason::Missing)
        }
        None => TargetPreparation::Create,
    }
}

impl QdrantIndex {
    /// Whether the error is the typed refusal of a rebuild target collection.
    ///
    /// The refusal is typed so the caller can release its own durable barrier
    /// and derive a fresh target name, instead of adopting a collection that
    /// is not provably its own.
    pub fn is_target_collision(error: &anyhow::Error) -> bool {
        error.downcast_ref::<RebuildTargetCollision>().is_some()
    }
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
            .ensure_collection(dimensions)
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

    /// Attaches to an existing collection with read-only validation only.
    ///
    /// Recovery must never mutate a reachable Qdrant: this performs
    /// `collection_exists`, `get`, and `count` and nothing else. A missing
    /// collection, a dimension mismatch, or an unreadable collection is
    /// reported to the caller, which decides whether creation is permitted for
    /// a genuinely uninitialized installation.
    pub async fn attach_read_only(config: &QdrantConfig, dimensions: usize) -> Result<Self> {
        Self::attach_read_only_collection(config, &config.collection_name, dimensions).await
    }

    /// Attaches to one named existing collection with read-only validation
    /// only.
    ///
    /// The collection is named explicitly because a rebuild target is a
    /// physical collection of its own, not the collection the settings name.
    pub async fn attach_read_only_collection(
        config: &QdrantConfig,
        collection_name: &str,
        dimensions: usize,
    ) -> Result<Self> {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        };
        index.probe_collection().await?;
        Ok(index)
    }

    /// Read-only inspection of the configured collection.
    ///
    /// Returns `None` when the collection does not exist, and otherwise the
    /// declared vector size and the current point count. Startup uses this to
    /// distinguish an uninitialized installation (no collection, or a genuinely
    /// empty one) from a populated collection that has no recorded identity, so
    /// it never adopts unknown vectors as an empty space.
    pub async fn inspect_existing(
        config: &QdrantConfig,
        dimensions: usize,
    ) -> Result<Option<(u64, usize)>> {
        Self::inspect_collection(config, &config.collection_name, dimensions).await
    }

    /// Read-only inspection of one named collection.
    ///
    /// The same contract as [`Self::inspect_existing`], for a physical
    /// collection that is not the one the settings name.
    pub async fn inspect_collection(
        config: &QdrantConfig,
        collection_name: &str,
        dimensions: usize,
    ) -> Result<Option<(u64, usize)>> {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        };
        if !index.collection_exists_checked().await? {
            return Ok(None);
        }
        let collection = index
            .client
            .collection_info(&index.collection_name)
            .await?
            .result
            .context("missing qdrant collection info")?;
        let actual_dimensions = collection_vector_size(&collection)
            .context("missing qdrant vector size in collection config")?;
        let points = index.count_points().await?;
        Ok(Some((points, actual_dimensions)))
    }

    /// Prepares the exact physical collection of one rebuild target.
    ///
    /// The durable ownership marker decides what may happen to the recorded
    /// collection. A marked target is one this attempt created: it is attached
    /// and probed, and a target of another dimension is refused. An unmarked
    /// target that already exists is ambiguous - the attempt cannot prove that
    /// it created that collection - so it is refused and never adopted. An
    /// absent unmarked target is created with the attempt dimension and probed.
    ///
    /// The returned flag says whether this call created the collection, so the
    /// caller can record the ownership marker directly after the create. No
    /// path deletes, empties or recreates a collection.
    pub async fn prepare_rebuild_target<F, Fut>(
        config: &QdrantConfig,
        collection_name: &str,
        dimensions: usize,
        created_by_owner: bool,
        authorize: F,
    ) -> Result<(Self, bool)>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        };
        let existing_dimensions = index.existing_dimensions().await?;
        match target_preparation(created_by_owner, existing_dimensions, dimensions) {
            TargetPreparation::Attach => {
                index.probe_collection().await?;
                Ok((index, false))
            }
            TargetPreparation::Create => {
                match index
                    .create_collection_guarded(dimensions, &authorize)
                    .await
                {
                    Ok(()) => {
                        index.probe_collection().await?;
                        Ok((index, true))
                    }
                    Err(error) if is_collection_already_exists(&error) => {
                        // A concurrent preparation created the same name. It is
                        // not provably the collection of this attempt, so it is
                        // refused instead of adopted.
                        Err(RebuildTargetCollision::new(
                            collection_name,
                            RebuildTargetCollisionReason::AlreadyExists,
                        )
                        .into())
                    }
                    Err(error) => Err(error),
                }
            }
            TargetPreparation::Collision(reason) => {
                Err(RebuildTargetCollision::new(collection_name, reason).into())
            }
        }
    }

    /// The declared dimension of the named collection, or `None` when it does
    /// not exist.
    async fn existing_dimensions(&self) -> Result<Option<usize>> {
        if !self.collection_exists_checked().await? {
            return Ok(None);
        }
        let collection = self
            .client
            .collection_info(&self.collection_name)
            .await?
            .result
            .context("missing qdrant collection info")?;
        Ok(Some(collection_vector_size(&collection).context(
            "missing qdrant vector size in collection config",
        )?))
    }

    /// Attaches to an existing collection, or creates it only when `allow_create`
    /// is true for a genuinely uninitialized collection.
    ///
    /// Creation is idempotent and non-destructive: a concurrent creator's
    /// `AlreadyExists` re-reads and validates the existing collection, and no
    /// path deletes or recreates anything. A mismatched existing collection is
    /// a configuration error; only the explicit rebuild path may recreate under
    /// its own safety gate.
    pub async fn bootstrap(
        config: &QdrantConfig,
        dimensions: usize,
        allow_create: bool,
    ) -> Result<(Self, bool)> {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: config.collection_name.clone(),
            dimensions,
        };
        if index.collection_exists_checked().await? {
            index.probe_collection().await?;
            return Ok((index, false));
        }
        if !allow_create {
            return Err(anyhow!(
                "qdrant collection {} is missing and this installation has prior vector state; refusing to create an empty replacement",
                index.collection_name
            ));
        }
        match index.create_collection(dimensions).await {
            Ok(()) => {
                index.probe_collection().await?;
                Ok((index, true))
            }
            Err(error) if is_collection_already_exists(&error) => {
                // Another replica won the create race: re-read and validate
                // instead of treating the race as a failure.
                index.probe_collection().await?;
                Ok((index, false))
            }
            Err(error) => Err(error),
        }
    }

    /// One read-only liveness/identity probe: collection exists, declared
    /// dimension matches, and a count round trip succeeds.
    pub async fn probe_collection(&self) -> Result<()> {
        if !self.collection_exists_checked().await? {
            return Err(anyhow!(
                "qdrant collection {} does not exist",
                self.collection_name
            ));
        }
        let collection = self
            .client
            .collection_info(&self.collection_name)
            .await?
            .result
            .context("missing qdrant collection info")?;
        let actual_dimensions = collection_vector_size(&collection)
            .context("missing qdrant vector size in collection config")?;
        if actual_dimensions != self.dimensions {
            return Err(anyhow!(
                "qdrant collection {} dimension mismatch: expected {}, found {}",
                self.collection_name,
                self.dimensions,
                actual_dimensions
            ));
        }
        self.count_points().await?;
        Ok(())
    }

    async fn collection_exists_checked(&self) -> Result<bool> {
        self.client
            .collection_exists(&self.collection_name)
            .await
            .map_err(|error| anyhow::Error::new(error).context("qdrant collection_exists failed"))
    }

    /// Ensures the configured collection exists at `dimensions`, never deleting
    /// or recreating one. A dimension mismatch is a configuration error: the
    /// legacy `recreate_on_dimension_mismatch` flag is accepted on the wire and
    /// in configuration but cannot authorize destruction.
    async fn ensure_collection(&self, dimensions: usize) -> Result<bool> {
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
                return Err(anyhow!(
                    "qdrant collection {} dimension mismatch: expected {}, found {}; a different \
                     vector space requires an explicit migration",
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

    /// Creates one rebuild target after re-proving its authority at every
    /// independent mutation boundary.
    ///
    /// Existence discovery and every preceding network await can outlive the
    /// grant that started the build, so the authority callback runs again
    /// immediately before the create RPC and before each payload-index
    /// mutation. A refusal at any boundary starts no new Qdrant mutation; a
    /// request already accepted by Qdrant is never promised to be rolled back.
    async fn create_collection_guarded<F, Fut>(
        &self,
        dimensions: usize,
        authorize: &F,
    ) -> Result<()>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        authorize().await?;
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ),
            )
            .await?;

        for &(field, field_type) in PAYLOAD_FIELD_INDEXES {
            authorize().await?;
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

fn is_collection_already_exists(error: &anyhow::Error) -> bool {
    let text = error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    text.contains("already exists") && text.contains("collection")
}

fn collection_vector_size(collection: &qdrant_client::qdrant::CollectionInfo) -> Option<usize> {
    let config = collection.config.as_ref()?;
    let vectors_config = config.params.as_ref()?.vectors_config.as_ref()?;

    match &vectors_config.config {
        Some(vectors_config::Config::Params(params)) => Some(params.size as usize),
        Some(vectors_config::Config::ParamsMap(params_map)) => params_map
            .map
            .values()
            .next()
            .map(|params| params.size as usize),
        None => None,
    }
}

#[cfg(test)]
impl QdrantIndex {
    /// A detached index for unit tests that never performs network I/O.
    pub(crate) fn for_test(collection_name: &str, dimensions: usize) -> Self {
        Self {
            client: Qdrant::from_url("http://127.0.0.1:1")
                .build()
                .expect("build test qdrant client"),
            collection_name: collection_name.to_string(),
            dimensions,
        }
    }
}

#[cfg(test)]
mod tests;
