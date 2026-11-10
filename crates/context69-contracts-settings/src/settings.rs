use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use context69_contracts_search::SearchMode;

pub use context69_contracts_search::{SEARCH_KEYWORD_WEIGHT_DEFAULT, SEARCH_VECTOR_WEIGHT_DEFAULT};

mod connection_tests;

pub use connection_tests::{
    TestLlmProviderOutcome, TestLlmProviderRequest, TestLlmProviderResponse,
    TestRuntimeEmbeddingPurpose, TestRuntimeEmbeddingRequest, TestRuntimeQdrantRequest,
    TestRuntimeValkeyRequest,
};

fn default_search_vector_weight() -> f32 {
    SEARCH_VECTOR_WEIGHT_DEFAULT
}

fn default_search_keyword_weight() -> f32 {
    SEARCH_KEYWORD_WEIGHT_DEFAULT
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchSettingsResponse {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    pub has_api_key: bool,
    /// Hybrid fusion weight for the semantic/vector channel in [0, 1].
    pub vector_weight: f32,
    /// Hybrid fusion weight for the keyword channel in [0, 1].
    pub keyword_weight: f32,
}

/// Explicit tri-state for PATCH-like secret updates.
///
/// Replaces the v0.15 `api_key + clear_api_key` dual flags: `Keep` leaves the
/// stored secret untouched, `Set` stores a new value, `Clear` removes it.
/// `Keep` is the default so missing fields never rotate or drop secrets.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum SecretPatch {
    #[default]
    Keep,
    Set(String),
    Clear,
}

impl SecretPatch {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    pub fn apply_to(&self, current: Option<String>) -> Option<String> {
        match self {
            Self::Keep => current,
            Self::Set(value) => Some(value.clone()),
            Self::Clear => None,
        }
    }
}

/// v0.15 search settings update kept for wire compatibility.
///
/// Deprecated `api_key`/`clear_api_key` dual flags stay; new code should use
/// [`CanonicalUpdateSearchSettingsRequest`], which carries a single
/// [`SecretPatch`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSearchSettingsRequest {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    /// Hybrid fusion weight for the semantic/vector channel; must be in [0, 1]
    /// and may not push `vector_weight + keyword_weight` above 1.
    #[serde(default = "default_search_vector_weight")]
    pub vector_weight: f32,
    /// Hybrid fusion weight for the keyword channel; must be in [0, 1] and may
    /// not push `vector_weight + keyword_weight` above 1.
    #[serde(default = "default_search_keyword_weight")]
    pub keyword_weight: f32,
}

/// v0.16 canonical search settings update: the `api_key`/`clear_api_key`
/// dual flags collapse into one [`SecretPatch`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CanonicalUpdateSearchSettingsRequest {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    #[serde(default)]
    pub api_key: SecretPatch,
    #[serde(default = "default_search_vector_weight")]
    pub vector_weight: f32,
    #[serde(default = "default_search_keyword_weight")]
    pub keyword_weight: f32,
}

impl CanonicalUpdateSearchSettingsRequest {
    pub fn secret_patch(&self) -> &SecretPatch {
        &self.api_key
    }
}

impl From<CanonicalUpdateSearchSettingsRequest> for UpdateSearchSettingsRequest {
    fn from(canonical: CanonicalUpdateSearchSettingsRequest) -> Self {
        let (api_key, clear_api_key) = match canonical.api_key {
            SecretPatch::Keep => (None, false),
            SecretPatch::Set(value) => (Some(value), false),
            SecretPatch::Clear => (None, true),
        };
        Self {
            mode: canonical.mode,
            rerank_enabled: canonical.rerank_enabled,
            rerank_base_url: canonical.rerank_base_url,
            rerank_model: canonical.rerank_model,
            candidate_limit: canonical.candidate_limit,
            timeout_secs: canonical.timeout_secs,
            api_key,
            clear_api_key,
            vector_weight: canonical.vector_weight,
            keyword_weight: canonical.keyword_weight,
        }
    }
}

impl From<UpdateSearchSettingsRequest> for CanonicalUpdateSearchSettingsRequest {
    fn from(legacy: UpdateSearchSettingsRequest) -> Self {
        let api_key = if legacy.clear_api_key {
            SecretPatch::Clear
        } else {
            match legacy.api_key.as_deref().map(str::trim) {
                Some(value) if !value.is_empty() => SecretPatch::Set(value.to_string()),
                _ => SecretPatch::Keep,
            }
        };
        Self {
            mode: legacy.mode,
            rerank_enabled: legacy.rerank_enabled,
            rerank_base_url: legacy.rerank_base_url,
            rerank_model: legacy.rerank_model,
            candidate_limit: legacy.candidate_limit,
            timeout_secs: legacy.timeout_secs,
            api_key,
            vector_weight: legacy.vector_weight,
            keyword_weight: legacy.keyword_weight,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSettingsResponse {
    pub qdrant: RuntimeQdrantSettings,
    pub embedding: RuntimeEmbeddingSettings,
    /// The optional, independent Git code embedding configuration.
    ///
    /// Absent when the deployment never stored a code row. A present value with
    /// `enabled = false` is an explicitly disabled configuration that retains
    /// its non-secret fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_embedding: Option<RuntimeCodeEmbeddingSettings>,
    pub scheduler: RuntimeSchedulerSettings,
    pub chunking: RuntimeChunkingSettings,
    pub file_library: RuntimeFileLibrarySettings,
    /// The verified local tokenizer artifacts this process can select.
    #[serde(default)]
    pub tokenizer_artifacts: Vec<TokenizerArtifactDescriptor>,
    /// The known tokenizer presets this deployment can prepare.
    ///
    /// A preset is a pinned upstream file, not an installed artifact: `ready`
    /// tells whether this process already holds a verified engine for it.
    #[serde(default)]
    pub tokenizer_presets: Vec<TokenizerPresetDescriptor>,
    /// True when the persisted vector identity does not match the stored
    /// embedding configuration, so existing vectors must be re-embedded through
    /// an explicit rebuild. Never inferred from connectivity alone.
    pub requires_rebuild: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeSettingsRequest {
    pub qdrant: RuntimeQdrantSettings,
    pub embedding: UpdateRuntimeEmbeddingSettings,
    /// The optional code embedding block.
    ///
    /// An absent or `null` field skips every code write, preserving the stored
    /// row and credential, so an older client cannot disable code embedding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_embedding: Option<UpdateCodeEmbeddingSettings>,
    pub scheduler: RuntimeSchedulerSettings,
    pub chunking: RuntimeChunkingSettings,
    pub file_library: UpdateRuntimeFileLibrarySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeQdrantSettings {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeEmbeddingSettings {
    pub base_url: String,
    /// The model documents are embedded with.
    pub model: String,
    /// The model queries are embedded with. Absent means the document model.
    #[serde(default)]
    pub query_model: Option<String>,
    pub dimensions: usize,
    pub timeout_secs: u64,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeEmbeddingSettings {
    pub base_url: String,
    /// The model documents are embedded with.
    pub model: String,
    /// The model queries are embedded with. An absent or blank value means the
    /// document model, so a deployment that never configures one keeps its
    /// current behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_model: Option<String>,
    pub dimensions: usize,
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Operator assertion that a changed gateway/model alias still produces the
    /// same vector space, so existing vectors may be kept. Transient: it is
    /// never stored as a runtime setting.
    #[serde(default)]
    pub confirm_vector_space_unchanged: bool,
}

/// The stored optional Git code embedding settings.
///
/// This block is independent of the document embedding block: it never borrows
/// the document model, credential, or object. `has_api_key` is presence
/// metadata only; the credential is never returned.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeCodeEmbeddingSettings {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    /// The model code queries are embedded with. Absent means the code model.
    #[serde(default)]
    pub query_model: Option<String>,
    pub dimensions: usize,
    pub timeout_secs: u64,
    pub has_api_key: bool,
}

/// The optional code embedding update.
///
/// The tri-state secret patch follows the settings contract: `keep` retains the
/// stored code credential, `set` replaces it, and `clear` removes it. Disabling
/// the block keeps every non-secret field and the credential unless the caller
/// explicitly clears the key.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateCodeEmbeddingSettings {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_model: Option<String>,
    pub dimensions: usize,
    pub timeout_secs: u64,
    #[serde(default)]
    pub api_key: SecretPatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSchedulerSettings {
    pub interval_secs: u64,
    pub run_on_start: bool,
    pub max_concurrency: usize,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valkey_url: Option<String>,
}

/// The durable state of the vector index rebuild.
///
/// The states are the rebuild's own, not the task queue's: a rebuild that
/// waits for the queue is still `Queued`, and a rebuild that waits for an
/// operator is `Paused` or `Pausing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VectorIndexRebuildState {
    Idle,
    /// The rebuild is accepted and waits for the queue or for a worker.
    Queued,
    Running,
    /// A pause was requested; the committed page still finishes.
    Pausing,
    /// The rebuild is parked and waits for an explicit resume.
    Paused,
    Succeeded,
    Failed,
    /// The rebuild was cancelled; its target was never activated.
    Cancelled,
}

/// The durable status of the vector index rebuild.
///
/// The optional fields are the durable facts an operator needs beside the
/// state: the task the controls bind to, the active and desired dimensions,
/// the physical collection that serves today and the one a running build
/// writes into. Every one of them is absent when the deployment has not
/// recorded it, so an older client and an older deployment keep working.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VectorIndexRebuildStatus {
    pub state: VectorIndexRebuildState,
    pub processed_chunks: usize,
    pub total_chunks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    /// The rebuild task the controls act on.
    ///
    /// A control request carries this id, so an operator can never act on a
    /// rebuild the status they observed is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    /// The dimensions of the active collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_dimensions: Option<usize>,
    /// The dimensions the deployment is configured for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_dimensions: Option<usize>,
    /// The physical collection ordinary writers use today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_collection: Option<String>,
    /// The physical collection a running build writes into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_collection: Option<String>,
    /// The model the active collection was built with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    /// Whether a pause would be accepted right now.
    #[serde(default)]
    pub can_pause: bool,
    /// Whether a resume would be accepted right now.
    #[serde(default)]
    pub can_resume: bool,
    /// Whether a cancellation would be accepted right now.
    #[serde(default)]
    pub can_cancel: bool,
    /// Whether the global task queue is paused.
    ///
    /// A queued rebuild then waits for the queue, not for the rebuild
    /// controls.
    #[serde(default)]
    pub queue_paused: bool,
}

/// The request one rebuild control carries.
///
/// The observed task id is the only input: the control acts on the rebuild
/// that task owns, and on no other one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RebuildControlRequest {
    pub task_id: Uuid,
}

/// Unit the chunking budget is measured in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChunkingUnit {
    /// Character budget. This is the legacy behavior.
    #[default]
    Chars,
    /// Token budget measured with the selected tokenizer.
    Tokens,
}

/// Tokenizer that measures the token budget.
///
/// The count only matches the tokenizer that produced it, so a token count is
/// never a count of another model's tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChunkingTokenizer {
    /// The `cl100k_base` tokenizer.
    #[default]
    Cl100kBase,
    /// The `o200k_base` tokenizer.
    O200kBase,
    /// One operator-provisioned local Hugging Face `tokenizer.json`.
    HfLocal,
}

/// One verified local tokenizer artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TokenizerArtifactDescriptor {
    /// The lowercase SHA256 of the artifact bytes, which is also its file name.
    pub sha256: String,
    /// The artifact size in bytes.
    pub size_bytes: u64,
}

/// One known tokenizer preset.
///
/// A preset describes one pinned upstream `tokenizer.json`. It is not an
/// installed artifact: `ready` tells whether this process already holds a
/// verified engine for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TokenizerPresetDescriptor {
    /// The stable preset identifier a provisioning request names.
    pub id: String,
    /// The operator-facing label.
    pub label: String,
    /// The lowercase SHA256 of the pinned file bytes.
    pub sha256: String,
    /// The expected size of the pinned file in bytes.
    pub size_bytes: u64,
    /// The license identifier the upstream project publishes.
    pub license: String,
    /// The official license URL.
    pub license_url: String,
    /// The upstream repository the preset is pinned to.
    pub source_repo: String,
    /// The official source URL of the pinned revision.
    pub source_url: String,
    /// The pinned upstream revision.
    pub revision: String,
    /// True when this process already holds a verified engine for the preset.
    pub ready: bool,
}

/// The outcome of one tokenizer preset provisioning request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerProvisionStatus {
    /// The file was fetched, verified, and activated by this request.
    Provisioned,
    /// A verified file was already cached and has now been activated.
    AlreadyPresent,
    /// The preset was already active in this process.
    AlreadyReady,
}

/// The request that prepares one known tokenizer preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TokenizerProvisionRequest {
    /// The known preset identifier to prepare.
    pub preset_id: String,
}

/// The outcome of preparing one known tokenizer preset.
///
/// The response never discloses a filesystem path or a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TokenizerProvisionResponse {
    /// The preset that was prepared.
    pub preset_id: String,
    /// The lowercase SHA256 of the verified file bytes.
    pub sha256: String,
    /// The size of the verified file in bytes.
    pub size_bytes: u64,
    /// What the request did.
    pub status: TokenizerProvisionStatus,
    /// True only when a verified engine is usable right now.
    pub ready: bool,
    /// The license identifier the upstream project publishes.
    pub license: String,
}

/// Token budget of one chunk when the unit is `tokens`.
pub const CHUNKING_MAX_TOKENS_DEFAULT: usize = 1024;

/// Token overlap between two chunks when the unit is `tokens`.
pub const CHUNKING_OVERLAP_TOKENS_DEFAULT: usize = 128;

fn default_chunking_max_tokens() -> usize {
    CHUNKING_MAX_TOKENS_DEFAULT
}

fn default_chunking_overlap_tokens() -> usize {
    CHUNKING_OVERLAP_TOKENS_DEFAULT
}

/// Chunking budget of the runtime settings.
///
/// The character values keep their own meaning: they are never relabeled as
/// token counts. The token values are only used while the unit is `tokens`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeChunkingSettings {
    /// Character budget of one chunk.
    pub max_chars: usize,
    /// Character overlap between two chunks.
    pub overlap_chars: usize,
    /// Unit the budget is measured in.
    #[serde(default)]
    pub unit: ChunkingUnit,
    /// Tokenizer that measures the token budget.
    #[serde(default)]
    pub tokenizer: ChunkingTokenizer,
    /// The pinned local artifact hash. Required for `hf_local`, absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_sha256: Option<String>,
    /// Token budget of one chunk, used while the unit is `tokens`.
    ///
    /// Stored in a PostgreSQL BIGINT, so the value must fit an `i64`.
    #[serde(default = "default_chunking_max_tokens")]
    #[schema(maximum = 9_223_372_036_854_775_807i64)]
    pub max_tokens: usize,
    /// Token overlap between two chunks, used while the unit is `tokens`.
    ///
    /// Stored in a PostgreSQL BIGINT, so the value must fit an `i64`.
    #[serde(default = "default_chunking_overlap_tokens")]
    #[schema(maximum = 9_223_372_036_854_775_807i64)]
    pub overlap_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeFileLibrarySettings {
    pub storage_root: String,
    pub max_upload_size_mb: usize,
    pub trusted_proxy_enabled: bool,
    /// The number of point ids the whole deployment may retain.
    pub registry_max_ids_total: i64,
    /// The number of point ids one file may retain.
    pub registry_max_ids_per_file: i64,
    /// The number of point ids one ingestion run may admit.
    pub registry_max_ids_per_run: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<RuntimeS3SettingsResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeFileLibrarySettings {
    pub storage_root: String,
    pub max_upload_size_mb: usize,
    #[serde(default)]
    pub trusted_proxy_enabled: bool,
    /// The number of point ids the whole deployment may retain.
    ///
    /// A client that omits the setting keeps the stored cap, so an older
    /// client never resets the policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_max_ids_total: Option<i64>,
    /// The number of point ids one file may retain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_max_ids_per_file: Option<i64>,
    /// The number of point ids one ingestion run may admit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_max_ids_per_run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<UpdateRuntimeS3Settings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeS3SettingsResponse {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub access_key: String,
    pub has_secret_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeS3Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub path_style: bool,
    pub access_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoclingSettingsSource {
    Config,
    Database,
    Unconfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingSettingsResponse {
    pub configured: bool,
    pub source: DoclingSettingsSource,
    pub connection: DoclingConnectionSettingsResponse,
    pub vlm: DoclingVlmSettingsResponse,
    /// The selected Docling OCR preset. A fresh or unconfigured installation
    /// reports the default preset.
    pub ocr_preset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateDoclingSettingsRequest {
    pub connection: UpdateDoclingConnectionSettings,
    #[serde(default)]
    pub vlm: UpdateDoclingVlmSettings,
    /// The selected Docling OCR preset.
    ///
    /// An absent field keeps the stored selection, so a client that does not
    /// know the setting never resets it. An explicit `null` is rejected
    /// instead of being read as an absent field.
    ///
    /// The generated schema documents the property as a plain optional string:
    /// the wire form keeps an explicit `null` distinguishable so it can be
    /// rejected, but `null` is not a value the API accepts, and the schema
    /// must not advertise one.
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(nullable = false)]
    pub ocr_preset: Option<Option<String>>,
}

/// Keeps an absent optional field distinguishable from an explicit `null`.
///
/// `Option<Option<T>>` alone collapses both into `None`, because serde maps a
/// JSON `null` onto the outer `None`. This deserializer runs only for a field
/// that is present, so an absent field keeps the `default` (`None`), an
/// explicit `null` becomes `Some(None)`, and a value becomes `Some(Some(..))`.
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The Docling OCR preset a fresh or unconfigured installation uses.
pub const DOCLING_OCR_PRESET_DEFAULT: &str = "rapidocr";
/// The longest accepted OCR preset identifier.
pub const DOCLING_OCR_PRESET_MAX_LEN: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingConnectionSettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub timeout_secs: u64,
    pub poll_interval_secs: u64,
    pub task_timeout_secs: u64,
    pub max_inflight: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateDoclingConnectionSettings {
    pub base_url: String,
    pub timeout_secs: u64,
    pub poll_interval_secs: u64,
    pub task_timeout_secs: u64,
    #[serde(default = "default_docling_max_inflight")]
    pub max_inflight: usize,
}

/// Persistent Docling remote admission ceiling (issue #118, lowered to a
/// single slot by issue #529).
///
/// The default of 1 matches the single blocking task worker
/// (`runtime.scheduler.max_concurrency = 1`): one local worker maps to one
/// remote slot, so the admission gate no longer churns. The bound stays
/// intentionally small (1..=32) to keep remote backpressure effective while
/// still allowing larger hosts to raise it via settings.
pub const DOCLING_MAX_INFLIGHT_DEFAULT: usize = 1;
pub const DOCLING_MAX_INFLIGHT_MIN: usize = 1;
pub const DOCLING_MAX_INFLIGHT_MAX: usize = 32;

fn default_docling_max_inflight() -> usize {
    DOCLING_MAX_INFLIGHT_DEFAULT
}

#[cfg(test)]
mod tests {
    use super::{
        DOCLING_MAX_INFLIGHT_DEFAULT, DOCLING_OCR_PRESET_DEFAULT, DoclingSettingsResponse,
        UpdateDoclingSettingsRequest, default_docling_max_inflight,
    };
    use utoipa::PartialSchema;

    /// The generated schema of one request, as the OpenAPI document carries it.
    fn request_schema() -> serde_json::Value {
        serde_json::to_value(UpdateDoclingSettingsRequest::schema())
            .expect("the request schema serializes")
    }

    /// The property names one schema requires.
    fn required_properties(schema: &serde_json::Value) -> Vec<String> {
        schema["required"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn docling_max_inflight_defaults_to_single_worker() {
        assert_eq!(DOCLING_MAX_INFLIGHT_DEFAULT, 1);
        assert_eq!(default_docling_max_inflight(), 1);
    }

    #[test]
    fn the_default_ocr_preset_is_rapidocr() {
        assert_eq!(DOCLING_OCR_PRESET_DEFAULT, "rapidocr");
    }

    /// An old client that does not know the OCR preset must be able to leave
    /// the stored selection untouched, so an absent field and an explicit
    /// `null` stay distinguishable on the wire.
    #[test]
    fn an_absent_ocr_preset_is_not_an_explicit_null() {
        let absent: UpdateDoclingSettingsRequest = serde_json::from_value(serde_json::json!({
            "connection": {
                "base_url": "http://docling:5001",
                "timeout_secs": 120,
                "poll_interval_secs": 2,
                "task_timeout_secs": 3600
            }
        }))
        .expect("a request without the ocr preset");
        assert_eq!(absent.ocr_preset, None);

        let explicit_null: UpdateDoclingSettingsRequest =
            serde_json::from_value(serde_json::json!({
                "connection": {
                    "base_url": "http://docling:5001",
                    "timeout_secs": 120,
                    "poll_interval_secs": 2,
                    "task_timeout_secs": 3600
                },
                "ocr_preset": null
            }))
            .expect("a request with an explicit null ocr preset");
        assert_eq!(explicit_null.ocr_preset, Some(None));

        let selected: UpdateDoclingSettingsRequest = serde_json::from_value(serde_json::json!({
            "connection": {
                "base_url": "http://docling:5001",
                "timeout_secs": 120,
                "poll_interval_secs": 2,
                "task_timeout_secs": 3600
            },
            "ocr_preset": "tesseract"
        }))
        .expect("a request with a selected ocr preset");
        assert_eq!(selected.ocr_preset, Some(Some("tesseract".to_string())));
    }

    /// The generated contract must document the update property as an optional
    /// string.
    ///
    /// The API rejects an explicit `null`, so a schema that advertises `null`
    /// would promise a value the API refuses. The property stays optional:
    /// an old client may omit it and keep its stored selection.
    #[test]
    fn the_ocr_preset_update_property_is_an_optional_string() {
        let schema = request_schema();
        let property = &schema["properties"]["ocr_preset"];
        assert_eq!(
            property["type"],
            serde_json::json!("string"),
            "the update property must be documented as a plain string, got: {property}"
        );
        assert!(
            property.get("nullable").is_none(),
            "the update property must not advertise null as a valid value, got: {property}"
        );
        assert!(
            !required_properties(&schema)
                .iter()
                .any(|name| name == "ocr_preset"),
            "the update property must stay optional, got: {schema}"
        );
    }

    /// The generated contract must document the response property as the
    /// selected preset, always present.
    #[test]
    fn the_ocr_preset_response_property_is_a_required_string() {
        let schema = serde_json::to_value(DoclingSettingsResponse::schema())
            .expect("the response schema serializes");
        let property = &schema["properties"]["ocr_preset"];
        assert_eq!(
            property["type"],
            serde_json::json!("string"),
            "the response property must be documented as a plain string, got: {property}"
        );
        assert!(
            required_properties(&schema)
                .iter()
                .any(|name| name == "ocr_preset"),
            "the response property must always be present, got: {schema}"
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingVlmSettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,
    pub has_api_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlm_pipeline_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_formula_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_preset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct UpdateDoclingVlmSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlm_pipeline_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_formula_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_preset: Option<String>,
}

#[cfg(test)]
mod rebuild_status_tests {
    use super::{RebuildControlRequest, VectorIndexRebuildState, VectorIndexRebuildStatus};
    use serde_json::json;

    #[test]
    fn the_rebuild_states_keep_their_wire_names() {
        for (state, wire) in [
            (VectorIndexRebuildState::Idle, "idle"),
            (VectorIndexRebuildState::Queued, "queued"),
            (VectorIndexRebuildState::Running, "running"),
            (VectorIndexRebuildState::Pausing, "pausing"),
            (VectorIndexRebuildState::Paused, "paused"),
            (VectorIndexRebuildState::Succeeded, "succeeded"),
            (VectorIndexRebuildState::Failed, "failed"),
            (VectorIndexRebuildState::Cancelled, "cancelled"),
        ] {
            assert_eq!(
                serde_json::to_value(state).expect("state serializes"),
                json!(wire)
            );
        }
    }

    #[test]
    fn an_older_status_payload_still_decodes_with_safe_defaults() {
        let status: VectorIndexRebuildStatus = serde_json::from_value(json!({
            "state": "running",
            "processed_chunks": 4,
            "total_chunks": 10,
        }))
        .expect("the pre-controls payload decodes");
        assert_eq!(status.state, VectorIndexRebuildState::Running);
        assert_eq!(status.task_id, None);
        assert_eq!(status.active_dimensions, None);
        assert_eq!(status.physical_collection, None);
        assert_eq!(status.target_collection, None);
        assert!(!status.can_pause);
        assert!(!status.can_resume);
        assert!(!status.can_cancel);
        assert!(!status.queue_paused);
    }

    #[test]
    fn a_control_request_is_strict_and_requires_the_observed_task() {
        let task_id = uuid::Uuid::new_v4();
        let request: RebuildControlRequest =
            serde_json::from_value(json!({ "task_id": task_id })).expect("the request decodes");
        assert_eq!(request.task_id, task_id);

        assert!(
            serde_json::from_value::<RebuildControlRequest>(json!({})).is_err(),
            "the observed task id is required"
        );
        assert!(
            serde_json::from_value::<RebuildControlRequest>(
                json!({ "task_id": task_id, "kind": "vector_rebuild" })
            )
            .is_err(),
            "an unknown field is refused instead of ignored"
        );
    }
}
