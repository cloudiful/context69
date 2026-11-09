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

/// Payload fields the separate Git code collection indexes, with their exact
/// Qdrant `FieldType`.
///
/// Canonical value types, not guessed numbers: the two UUID provenance fields
/// are stored as strings (Keyword), the counters and line numbers are integers,
/// and the path/language filters are strings.
pub(crate) const CODE_PAYLOAD_FIELD_INDEXES: &[(&str, FieldType)] = &[
    ("group_id", FieldType::Integer),
    ("repository_key", FieldType::Keyword),
    ("generation_key", FieldType::Keyword),
    ("file_key", FieldType::Keyword),
    ("path", FieldType::Keyword),
    ("language", FieldType::Keyword),
    ("chunk_index", FieldType::Integer),
    ("start_line", FieldType::Integer),
    ("end_line", FieldType::Integer),
];

/// The two collection shapes the shared ensure path serves.
///
/// The document shape tolerates Params or ParamsMap; the code shape requires
/// exactly one unnamed dense vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectionShape {
    Document,
    Code,
}

impl CollectionShape {
    fn payload_field_indexes(self) -> &'static [(&'static str, FieldType)] {
        match self {
            Self::Document => PAYLOAD_FIELD_INDEXES,
            Self::Code => CODE_PAYLOAD_FIELD_INDEXES,
        }
    }
}

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
            .ensure_collection(dimensions, CollectionShape::Document)
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
        let index = Self::connect_to(config, collection_name, dimensions)?;
        index.probe_collection().await?;
        Ok(index)
    }

    /// Attaches to one named existing code collection with the strict code
    /// vector-shape validation and read-only probe.
    ///
    /// A named-vector map, a wrong distance, or a wrong dimension is refused
    /// before any read; this never creates, mutates, or recreates the
    /// collection. The caller maps the refusal to its existing lexical
    /// fallback.
    pub(crate) async fn attach_code_read_only_collection(
        config: &QdrantConfig,
        collection_name: &str,
        dimensions: usize,
    ) -> Result<Self> {
        let index = Self::connect_to(config, collection_name, dimensions)?;
        index.probe_code_collection().await?;
        Ok(index)
    }

    /// Binds a client to one named collection at the expected dimension.
    fn connect_to(config: &QdrantConfig, collection_name: &str, dimensions: usize) -> Result<Self> {
        Ok(Self {
            client: Qdrant::from_url(&config.url).build()?,
            collection_name: collection_name.to_string(),
            dimensions,
        })
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
        Ok(Some(
            self.existing_dimension(CollectionShape::Document).await?,
        ))
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
        match index
            .create_collection(dimensions, CollectionShape::Document)
            .await
        {
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

    /// Ensures one named code collection exists at `dimensions` with its strict
    /// single-dense shape and code filter indexes, then probes it once.
    pub(crate) async fn ensure_code_collection(
        config: &QdrantConfig,
        collection_name: &str,
        dimensions: usize,
    ) -> Result<Self> {
        let index = Self::connect_to(config, collection_name, dimensions)?;
        index
            .ensure_collection(dimensions, CollectionShape::Code)
            .await?;
        index.probe_code_collection().await?;
        Ok(index)
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

    /// Strict code-only probe: one unnamed dense Cosine vector at the expected
    /// dimension, then a count round trip. It never mutates the collection.
    async fn probe_code_collection(&self) -> Result<()> {
        if !self.collection_exists_checked().await? {
            return Err(anyhow!(
                "qdrant collection {} does not exist",
                self.collection_name
            ));
        }
        self.validate_dimension(CollectionShape::Code, self.dimensions)
            .await?;
        self.count_points().await?;
        Ok(())
    }

    async fn collection_exists_checked(&self) -> Result<bool> {
        self.client
            .collection_exists(&self.collection_name)
            .await
            .map_err(|error| anyhow::Error::new(error).context("qdrant collection_exists failed"))
    }

    /// Ensures one collection exists at `dimensions` with the shape's payload
    /// indexes, never deleting or recreating one. The document path only
    /// validates its tolerant vector shape; the code path additionally
    /// reconciles indexes and accepts a concurrent creator only after
    /// revalidation. Callers that need a liveness probe add it themselves.
    async fn ensure_collection(&self, dimensions: usize, shape: CollectionShape) -> Result<bool> {
        if self.collection_exists_checked().await? {
            self.validate_dimension(shape, dimensions).await?;
            if shape == CollectionShape::Code {
                self.ensure_field_indexes(shape.payload_field_indexes())
                    .await?;
            }
            return Ok(false);
        }

        match self.create_collection(dimensions, shape).await {
            Ok(()) => Ok(true),
            Err(error)
                if shape == CollectionShape::Code && is_collection_already_exists(&error) =>
            {
                // A concurrent creator owns the name now: revalidate instead of
                // adopting it as this attempt's create.
                self.validate_dimension(shape, dimensions).await?;
                self.ensure_field_indexes(shape.payload_field_indexes())
                    .await?;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// Rejects a collection whose declared dimension differs from the shape's
    /// expected one, using that shape's existing message.
    async fn validate_dimension(&self, shape: CollectionShape, expected: usize) -> Result<()> {
        let actual = self.existing_dimension(shape).await?;
        if actual != expected {
            return Err(collection_dimension_mismatch(
                shape,
                &self.collection_name,
                expected,
                actual,
            ));
        }
        Ok(())
    }

    /// The declared dimension of the collection under the shape's vector rule.
    async fn existing_dimension(&self, shape: CollectionShape) -> Result<usize> {
        let collection = self
            .client
            .collection_info(&self.collection_name)
            .await?
            .result
            .context("missing qdrant collection info")?;
        match shape {
            CollectionShape::Document => collection_vector_size(&collection)
                .context("missing qdrant vector size in collection config"),
            CollectionShape::Code => code_vector_size_single(&collection)
                .context("code qdrant collection is not a single Cosine dense vector collection"),
        }
    }

    async fn create_collection(&self, dimensions: usize, shape: CollectionShape) -> Result<()> {
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ),
            )
            .await?;
        self.ensure_field_indexes(shape.payload_field_indexes())
            .await
    }

    async fn ensure_field_indexes(&self, indexes: &[(&str, FieldType)]) -> Result<()> {
        for &(field, field_type) in indexes {
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

fn collection_dimension_mismatch(
    shape: CollectionShape,
    collection_name: &str,
    expected: usize,
    actual: usize,
) -> anyhow::Error {
    match shape {
        CollectionShape::Document => anyhow!(
            "qdrant collection {} dimension mismatch: expected {}, found {}; a different \
             vector space requires an explicit migration",
            collection_name,
            expected,
            actual
        ),
        CollectionShape::Code => anyhow!(
            "qdrant code collection {} dimension mismatch: expected {}, found {}",
            collection_name,
            expected,
            actual
        ),
    }
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

/// The size of the single unnamed dense Cosine vector a code collection must
/// declare.
///
/// A named-vector map, a non-Cosine distance, a multi-vector configuration,
/// or a missing configuration is a different collection type and is refused
/// instead of silently reading one of its vectors.
fn code_vector_size_single(collection: &qdrant_client::qdrant::CollectionInfo) -> Option<usize> {
    let config = collection.config.as_ref()?;
    let vectors_config = config.params.as_ref()?.vectors_config.as_ref()?;
    match &vectors_config.config {
        Some(vectors_config::Config::Params(params))
            if params.distance == Distance::Cosine as i32
                && params.multivector_config.is_none() =>
        {
            Some(params.size as usize)
        }
        Some(vectors_config::Config::Params(_))
        | Some(vectors_config::Config::ParamsMap(_))
        | None => None,
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
