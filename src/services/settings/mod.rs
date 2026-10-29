use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::{
    db::StoredRuntimeSettings,
    domain_errors::DomainError,
    embedding::{
        EmbeddingIdentity, EmbeddingProvider, build_provider,
        embedding_runtime::vector_identity_fingerprint,
    },
    vector_runtime::VectorIdentityAck,
};

mod docling_probe;
mod embedding_probe;
pub mod llm_probe;
mod mappers;
pub mod qdrant_probe;
mod runtime_mappers;
pub(crate) mod secrets;
mod validate;

use self::{
    mappers::{
        canonical_search_settings_from_request, config_from_stored, docling_settings_from_request,
        resolved_ocr_preset, response_from_stored, search_response_from_stored,
        search_settings_from_request, unconfigured_docling_response, validate_docling_vlm_shape,
    },
    runtime_mappers::{
        default_runtime_settings_response, runtime_settings_from_request,
        runtime_settings_response, s3_secret_patch,
    },
    secrets::{SettingsSecrets, docling_vlm_patch, optional_key_patch},
    validate as settings_validate,
};

use crate::{
    contracts::{
        CanonicalUpdateSearchSettingsRequest, DoclingSettingsResponse, DoclingSettingsSource,
        RuntimeSettingsResponse, SearchSettingsResponse, UpdateDoclingSettingsRequest,
        UpdateRuntimeEmbeddingSettings, UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
    },
    db::{Database, default_search_settings},
    docling::DoclingConfig,
    services::secret_store::{self, SecretStore},
    support::normalize::normalize_optional_string,
};

/// Rejects an embedding update the live runtime cannot apply, before it is
/// persisted.
type RuntimeEmbeddingGuard =
    Arc<dyn Fn(&UpdateRuntimeEmbeddingSettings) -> Result<()> + Send + Sync>;

/// Applies a committed, confirmed same-space identity transition to the live
/// runtime before the save reports success. The provider is already built, so a
/// failure here is a runtime reconciliation failure, not a settings validation
/// failure.
pub type RuntimeEmbeddingApplier = Arc<
    dyn Fn(
            Arc<dyn EmbeddingProvider>,
            EmbeddingIdentity,
            StoredRuntimeSettings,
            Option<String>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// The outcome of one authoritative runtime-configuration refresh.
///
/// A deferred refresh is distinguishable from an applied one, so a caller never
/// reports "not now" as "done".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigRefreshOutcome {
    /// Applied and the vector-relevant configuration advanced the generation.
    Applied,
    /// Applied without changing the vector-relevant configuration.
    AppliedUnchanged,
    /// A rebuild or another transition owns the runtime; retry later.
    Deferred,
    /// The authoritative configuration itself cannot be built; retrying cannot
    /// help until it is saved again.
    Rejected,
}

/// Loads the authoritative persisted configuration and applies it to the live
/// runtime. It runs while the settings save lock and the shared runtime
/// transition are held, so it always observes and installs one committed state.
pub(crate) type RuntimeConfigRefresher = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<ConfigRefreshOutcome>> + Send>> + Send + Sync,
>;

/// The shared identity-transition exclusion. Beginning it returns `false` when
/// a reload, rebuild, or another identity save already owns it, so the save is
/// rejected before any secret or settings write.
#[derive(Clone)]
pub struct RuntimeEmbeddingTransition {
    begin: Arc<dyn Fn() -> bool + Send + Sync>,
    end: Arc<dyn Fn() + Send + Sync>,
}

impl RuntimeEmbeddingTransition {
    pub fn new(
        begin: impl Fn() -> bool + Send + Sync + 'static,
        end: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            begin: Arc::new(begin),
            end: Arc::new(end),
        }
    }

    fn begin(&self) -> bool {
        (self.begin)()
    }

    fn end(&self) {
        (self.end)()
    }
}

struct TransitionHold(RuntimeEmbeddingTransition);

impl Drop for TransitionHold {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// The persisted identity a confirmed same-space save would acknowledge.
struct PlannedAck {
    collection_name: String,
    expected_fingerprint: String,
    expected_dimensions: usize,
    candidate_fingerprint: String,
    embedding_base_url: String,
    embedding_model: String,
}

impl PlannedAck {
    fn as_ack(&self) -> VectorIdentityAck<'_> {
        VectorIdentityAck {
            collection_name: &self.collection_name,
            expected_fingerprint: &self.expected_fingerprint,
            expected_dimensions: self.expected_dimensions,
            candidate_fingerprint: &self.candidate_fingerprint,
            embedding_base_url: &self.embedding_base_url,
            embedding_model: &self.embedding_model,
        }
    }
}

/// The destination part of a Qdrant URL that must stay fixed while existing
/// vectors are acknowledged. Trailing slashes are not a different destination.
fn normalize_endpoint(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

#[derive(Clone)]
pub struct SettingsService {
    db: Database,
    search_secrets: SettingsSecrets,
    embedding_secrets: SettingsSecrets,
    docling_secrets: SettingsSecrets,
    s3_secrets: SettingsSecrets,
    llm_provider_secrets: SettingsSecrets,
    docling_settings_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    runtime_settings_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Rejects an embedding update that cannot be applied to the live runtime
    /// (a dimension change against a fixed-dimension vector index). Installed
    /// only by the running application, so an unconfigured service keeps the
    /// contract its callers already exercise.
    runtime_embedding_guard: Option<RuntimeEmbeddingGuard>,
    /// Applies a confirmed same-space identity transition to the live runtime.
    runtime_embedding_applier: Option<RuntimeEmbeddingApplier>,
    /// The shared exclusion an identity transition takes for the whole save.
    runtime_embedding_transition: Option<RuntimeEmbeddingTransition>,
    /// Reports a runtime-side rebuild requirement (populated vectors with no
    /// recorded identity) that the persisted row alone cannot express.
    runtime_requires_rebuild: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Reports an active explicit rebuild, so a destination change is refused
    /// while it owns a collection.
    runtime_rebuild_in_progress: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Serializes runtime settings saves across clones, so two saves cannot
    /// interleave their secret, settings, and runtime application steps. A
    /// deferred observer refresh takes the same lock, so it can never cross a
    /// save or acknowledgement.
    runtime_save_lock: Arc<Mutex<()>>,
    /// Loads and applies the authoritative runtime configuration for an observer
    /// notification. Installed only by the running application.
    runtime_config_refresher: Option<RuntimeConfigRefresher>,
    /// Set when a notification needs an authoritative refresh; cleared when the
    /// refresh runs. A request that lands mid-refresh keeps it pending.
    runtime_refresh_requested: Arc<AtomicBool>,
}

impl SettingsService {
    /// Builds the service on a store with no master key.
    ///
    /// Every API key then round-trips through the shared store in its legacy
    /// plaintext representation and is read back from the legacy column, which is
    /// exactly the behaviour of a deployment that has not configured a master
    /// key. The application path is [`Self::with_secrets`], which wires the
    /// configured store; this constructor exists for callers that have no
    /// deployment configuration to hand, and it uses one code path rather than a
    /// bypass.
    pub fn new(db: Database) -> Self {
        Self::with_secrets(db.clone(), secret_store::build_unkeyed(&db))
    }

    /// Builds the service on the configured shared store.
    ///
    /// The API-key categories each get their own binding, so a call site never
    /// names a secret by a string literal.
    pub fn with_secrets(db: Database, store: SecretStore) -> Self {
        Self {
            db,
            search_secrets: SettingsSecrets::search(store.clone()),
            embedding_secrets: SettingsSecrets::embedding(store.clone()),
            docling_secrets: SettingsSecrets::docling(store.clone()),
            s3_secrets: SettingsSecrets::runtime_s3(store.clone()),
            llm_provider_secrets: SettingsSecrets::new(
                store,
                crate::services::secret_store::SecretPurpose::TranslationProviderApiKey,
                crate::services::secret_store::key_names::TRANSLATION_PROVIDER_API_KEY,
            ),
            docling_settings_observer: None,
            runtime_settings_observer: None,
            runtime_embedding_guard: None,
            runtime_embedding_applier: None,
            runtime_embedding_transition: None,
            runtime_requires_rebuild: None,
            runtime_rebuild_in_progress: None,
            runtime_save_lock: Arc::new(Mutex::new(())),
            runtime_config_refresher: None,
            runtime_refresh_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Register a hook invoked after Docling settings are saved, so dependency
    /// gates (e.g. docling readiness) refresh without a process restart.
    pub fn set_docling_settings_observer(&mut self, observer: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.docling_settings_observer = observer;
    }

    /// Register a hook invoked after runtime settings are saved, so dependency
    /// gates (the S3 gate in particular) refresh and re-probe without a process
    /// restart. Like the Docling hook it is notification only: the persisted
    /// row is already committed when it fires.
    pub fn set_runtime_settings_observer(&mut self, observer: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.runtime_settings_observer = observer;
    }

    /// Register a guard that rejects an embedding update the live runtime cannot
    /// apply (an identity change while a fixed index is live, or a save racing a
    /// reload/rebuild). It runs before anything is persisted, so a rejected save
    /// leaves the stored row and the running runtime unchanged.
    pub fn set_runtime_embedding_guard(&mut self, guard: Option<RuntimeEmbeddingGuard>) {
        self.runtime_embedding_guard = guard;
    }

    /// Installs the awaited hot-apply hook for a confirmed same-space identity
    /// transition. The settings row is already committed when it runs, so its
    /// failure is reported to the caller as a runtime reconciliation failure.
    pub fn set_runtime_embedding_applier(&mut self, applier: Option<RuntimeEmbeddingApplier>) {
        self.runtime_embedding_applier = applier;
    }

    /// Installs the shared exclusion that keeps an identity transition, a live
    /// reload, and a manual rebuild start from overlapping.
    pub fn set_runtime_embedding_transition(
        &mut self,
        transition: Option<RuntimeEmbeddingTransition>,
    ) {
        self.runtime_embedding_transition = transition;
    }

    /// Installs the authoritative-configuration refresher used when a runtime
    /// settings observer requests a reconfiguration.
    pub(crate) fn set_runtime_config_refresher(
        &mut self,
        refresher: Option<RuntimeConfigRefresher>,
    ) {
        self.runtime_config_refresher = refresher;
    }

    /// Requests an authoritative runtime-configuration refresh. It carries no
    /// captured configuration: the refresh re-reads the committed row under the
    /// shared save lock.
    pub(crate) fn request_runtime_refresh(&self) {
        self.runtime_refresh_requested
            .store(true, Ordering::Release);
    }

    /// Applies the newest committed configuration to the live runtime.
    ///
    /// The settings save lock is taken first, then the shared runtime
    /// transition, then the authoritative row is read and applied. A save and a
    /// refresh therefore cannot cross: either the refresh completes before the
    /// save obtains the lock, or it observes the newer committed state. If the
    /// runtime is busy the locks are released and the refresh defers, so a
    /// rebuild never blocks credential repair behind a long-running operation.
    pub(crate) async fn refresh_runtime_configuration(&self) -> Result<ConfigRefreshOutcome> {
        refresh_runtime_configuration_under_lock(
            &self.runtime_save_lock,
            self.runtime_embedding_transition.as_ref(),
            self.runtime_config_refresher.as_ref(),
        )
        .await
    }

    /// Runs the refresh loop for observer notifications until shutdown.
    pub(crate) async fn run_runtime_refresh_loop(
        &self,
        notify: Arc<tokio::sync::Notify>,
        shutdown: tokio_util::sync::CancellationToken,
    ) {
        let requested = self.runtime_refresh_requested.clone();
        let service = self.clone();
        reconcile_runtime_refreshes(&requested, &notify, &shutdown, move || {
            let service = service.clone();
            Box::pin(async move { service.refresh_runtime_configuration().await })
        })
        .await;
    }

    /// Installs the runtime-side rebuild requirement, so a response stays
    /// truthful when the live runtime knows about a mismatch the persisted row
    /// cannot express.
    pub fn set_runtime_requires_rebuild(
        &mut self,
        resolver: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) {
        self.runtime_requires_rebuild = resolver;
    }

    /// Installs the active-rebuild probe, so a vector destination change is
    /// refused while a rebuild owns the admitted collection.
    pub fn set_runtime_rebuild_in_progress(
        &mut self,
        resolver: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) {
        self.runtime_rebuild_in_progress = resolver;
    }

    pub async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse> {
        let Some(stored) = self.db.get_runtime_settings().await? else {
            return Ok(default_runtime_settings_response());
        };
        // Presence only: a settings projection never opens a stored key, so it
        // stays truthful on a deployment that cannot decrypt.
        let has = self.embedding_secrets.is_present().await?;
        let has_s3_secret_key = match stored.file_library.s3.as_ref() {
            Some(_) => self.s3_secrets.is_present().await?,
            None => false,
        };
        let requires_rebuild = self.requires_rebuild(&stored).await?;
        Ok(runtime_settings_response(
            stored,
            has,
            has_s3_secret_key,
            requires_rebuild,
        ))
    }

    pub async fn trusted_proxy_enabled(&self) -> Result<bool> {
        Ok(self
            .db
            .get_runtime_settings()
            .await?
            .is_some_and(|settings| settings.file_library.trusted_proxy_enabled))
    }

    pub async fn update_runtime_settings(
        &self,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<RuntimeSettingsResponse> {
        settings_validate::runtime_settings_request(request)?;

        // Saves are serialized across clones, so the identity decision, the
        // settings transaction, and the hot apply are one ordered operation.
        let _save = self.runtime_save_lock.lock().await;

        // Rejected before any write when the live runtime cannot apply it (a
        // model/dimension/endpoint identity change against a live vector index),
        // so a save never reports success while leaving the old runtime in
        // effect.
        if let Some(guard) = &self.runtime_embedding_guard {
            guard(&request.embedding)?;
        }

        let patch = optional_key_patch(request.embedding.api_key.clone());
        // Resolved before anything is written even though the value is not part
        // of the row: a save must fail closed on a sealed key this deployment
        // cannot open, exactly as a read does.
        let embedding_api_key = self.embedding_secrets.resolve_and_merge(&patch).await?;
        let candidate_identity = EmbeddingIdentity::from_update(&request.embedding);

        let existing = self.db.get_runtime_settings().await?;
        // A rebuild owns the vectors of the collection it admitted. Changing the
        // Qdrant destination while it runs would make its completion describe a
        // different collection, even when the embedding identity is unchanged,
        // so the save is rejected before any secret or settings write.
        if let Some(in_progress) = &self.runtime_rebuild_in_progress
            && in_progress()
            && let Some(existing) = existing.as_ref()
        {
            let destination_changed = normalize_endpoint(&request.qdrant.url)
                != normalize_endpoint(&existing.qdrant.url)
                || request.qdrant.collection_name.trim() != existing.qdrant.collection_name.trim();
            if destination_changed {
                return Err(DomainError::conflict(
                    "runtime.qdrant destination cannot change while a vector index rebuild is in \
                     progress; retry after it completes",
                )
                .into());
            }
        }
        let planned_ack = self
            .plan_identity_acknowledgment(existing.as_ref(), &candidate_identity, request)
            .await?;
        if planned_ack.is_some() && self.runtime_embedding_applier.is_none() {
            return Err(DomainError::invalid_argument(
                "runtime.embedding identity cannot change without a running runtime to apply the \
                 acknowledged vector space",
            )
            .into());
        }
        // Build the candidate provider before the settings transaction and
        // before any secret write. Construction performs no network I/O, so a
        // confirmed transition never reaches the gateway while still deciding.
        let candidate_provider = match &planned_ack {
            Some(_) => Some(build_provider(&crate::config::EmbeddingConfig {
                base_url: request.embedding.base_url.trim().to_string(),
                api_key: embedding_api_key.clone(),
                model: request.embedding.model.trim().to_string(),
                dimensions: request.embedding.dimensions,
                timeout: std::time::Duration::from_secs(request.embedding.timeout_secs),
            })?),
            None => None,
        };

        // The S3 secret key follows the same contract as the API keys, and it is
        // the only way this request learns whether a key is in effect: a supplied
        // one replaces it, an absent or blank one keeps the stored one.
        let s3_patch = s3_secret_patch(request.file_library.s3.as_ref());
        let s3_secret_key = self.s3_secrets.resolve_and_merge(&s3_patch).await?;

        let stored = runtime_settings_from_request(request);
        if stored.file_library.s3.is_some() && s3_secret_key.is_none() {
            return Err(DomainError::invalid_argument(
                "runtime.file_library.s3.secret_key must not be empty",
            )
            .into());
        }

        // The identity transition owns the shared exclusion from here through
        // the hot apply, so a reload or manual rebuild start cannot interleave.
        let transition_hold = match (&planned_ack, &self.runtime_embedding_transition) {
            (Some(_), Some(transition)) => {
                if !transition.begin() {
                    return Err(DomainError::conflict(
                        "a runtime reload, rebuild, or identity transition is in progress; retry \
                         after it completes",
                    )
                    .into());
                }
                Some(TransitionHold(transition.clone()))
            }
            (Some(_), None) => {
                return Err(DomainError::invalid_argument(
                    "runtime.embedding identity cannot change without a running transition \
                     coordinator",
                )
                .into());
            }
            (None, _) => None,
        };

        // Committed only once the candidate is known to be valid, so a rejected
        // request leaves the stored keys as they were.
        self.embedding_secrets.commit(&patch).await?;
        self.s3_secrets.commit(&s3_patch).await?;
        let saved = match &planned_ack {
            Some(ack) => {
                self.db
                    .save_runtime_settings_with_identity_ack(&stored, Some(ack.as_ack()))
                    .await?
            }
            None => self.db.save_runtime_settings(&stored).await?,
        };

        // A confirmed same-space transition reports success only once the live
        // pair serves the acknowledged identity. A failure here withdraws
        // readiness and blocks further identity saves until reconciliation.
        if let (Some(_), Some(provider), Some(applier)) = (
            planned_ack.as_ref(),
            candidate_provider.clone(),
            self.runtime_embedding_applier.as_ref(),
        ) {
            applier(
                provider,
                candidate_identity.clone(),
                stored.clone(),
                embedding_api_key.clone(),
            )
            .await?;
        }

        // The hot apply is complete; release the exclusion before notifying the
        // observer so its dependency refresh is not mistaken for a racing
        // transition.
        drop(transition_hold);
        if let Some(observer) = &self.runtime_settings_observer {
            observer();
        }
        let has = self.embedding_secrets.is_present().await?;
        let has_s3_secret_key = match saved.file_library.s3.as_ref() {
            Some(_) => self.s3_secrets.is_present().await?,
            None => false,
        };
        let requires_rebuild = self.requires_rebuild(&saved).await?;
        Ok(runtime_settings_response(
            saved,
            has,
            has_s3_secret_key,
            requires_rebuild,
        ))
    }

    /// Whether the persisted vector identity still matches the stored embedding
    /// configuration. A missing persisted row is not a mismatch: it describes an
    /// installation whose vectors have not been built yet, not a stale space.
    async fn requires_rebuild(&self, stored: &StoredRuntimeSettings) -> Result<bool> {
        let stored_fingerprint = self
            .db
            .get_vector_index_fingerprint(&stored.qdrant.collection_name)
            .await?;
        if self
            .runtime_requires_rebuild
            .as_ref()
            .is_some_and(|resolver| resolver())
        {
            return Ok(true);
        }
        let Some(stored_fingerprint) = stored_fingerprint else {
            return Ok(false);
        };
        Ok(stored_fingerprint
            != vector_identity_fingerprint(
                stored.qdrant.url.trim(),
                stored.qdrant.collection_name.trim(),
                stored.embedding.base_url.trim(),
                stored.embedding.model.trim(),
                stored.embedding.dimensions,
            ))
    }

    /// Decides whether the request is a confirmed same-space identity
    /// transition, and whether the persisted identity may be acknowledged.
    ///
    /// An unconfirmed identity change, a dimension change, a collection change,
    /// or a missing/mismatched persisted identity is rejected here, before any
    /// secret or settings write.
    async fn plan_identity_acknowledgment(
        &self,
        existing: Option<&StoredRuntimeSettings>,
        candidate: &EmbeddingIdentity,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<Option<PlannedAck>> {
        let Some(existing) = existing else {
            // First configuration: there is no persisted vector space to
            // acknowledge, so the existing bootstrap path applies.
            return Ok(None);
        };
        let existing_identity = EmbeddingIdentity {
            base_url: existing
                .embedding
                .base_url
                .trim()
                .trim_end_matches('/')
                .to_string(),
            model: existing.embedding.model.trim().to_string(),
            dimensions: existing.embedding.dimensions,
        };
        if &existing_identity == candidate {
            // Credential, timeout, or non-embedding settings only.
            return Ok(None);
        }

        if candidate.dimensions != existing_identity.dimensions {
            return Err(DomainError::invalid_argument(
                "runtime.embedding.dimensions cannot change against existing vectors; a different \
                 vector space requires an explicit dimension migration",
            )
            .into());
        }
        if !request.embedding.confirm_vector_space_unchanged {
            return Err(DomainError::invalid_argument(
                "runtime.embedding model/base URL changes require an explicit confirmation that \
                 the new gateway/alias produces the same vector space",
            )
            .into());
        }

        // The acknowledgment names the vector destination as well as the
        // provider identity: a confirmed alias may never also move the vectors
        // to another Qdrant URL or collection.
        let collection_name = request.qdrant.collection_name.trim();
        if collection_name != existing.qdrant.collection_name.trim() {
            return Err(DomainError::invalid_argument(
                "runtime.qdrant.collection_name cannot change while acknowledging existing vectors",
            )
            .into());
        }
        if normalize_endpoint(&request.qdrant.url) != normalize_endpoint(&existing.qdrant.url) {
            return Err(DomainError::invalid_argument(
                "runtime.qdrant.url cannot change while acknowledging existing vectors; the \
                 acknowledgement describes the existing collection, not a different destination",
            )
            .into());
        }

        let expected_fingerprint = vector_identity_fingerprint(
            existing.qdrant.url.trim(),
            existing.qdrant.collection_name.trim(),
            &existing_identity.base_url,
            &existing_identity.model,
            existing_identity.dimensions,
        );
        let persisted = self
            .db
            .get_vector_index_fingerprint(collection_name)
            .await?;
        if persisted.as_deref() != Some(expected_fingerprint.as_str()) {
            return Err(DomainError::invalid_argument(
                "the persisted vector identity does not match the stored embedding settings; a \
                 confirmed same-space transition cannot acknowledge it",
            )
            .into());
        }

        Ok(Some(PlannedAck {
            collection_name: collection_name.to_string(),
            expected_fingerprint,
            expected_dimensions: existing_identity.dimensions,
            candidate_fingerprint: vector_identity_fingerprint(
                request.qdrant.url.trim(),
                collection_name,
                &candidate.base_url,
                &candidate.model,
                candidate.dimensions,
            ),
            embedding_base_url: candidate.base_url.clone(),
            embedding_model: candidate.model.clone(),
        }))
    }

    pub async fn test_s3_connection(
        &self,
        request: &crate::contracts::UpdateRuntimeS3Settings,
    ) -> Result<()> {
        // A probe never writes. A supplied key is used as given, and otherwise
        // the stored one is resolved through the store, which fails closed
        // rather than reporting no credential.
        let secret_key = match normalize_optional_string(request.secret_key.clone()) {
            Some(supplied) => supplied,
            None => self
                .s3_secrets
                .resolve()
                .await?
                .ok_or_else(|| {
                    DomainError::invalid_argument(
                        "runtime.file_library.s3.secret_key must not be empty",
                    )
                })
                .map_err(anyhow::Error::from)?,
        };
        let config = crate::config::S3StorageConfig {
            endpoint: request.endpoint.trim().to_string(),
            region: request.region.trim().to_string(),
            bucket: request.bucket.trim().to_string(),
            prefix: request.prefix.trim_matches('/').to_string(),
            path_style: request.path_style,
            access_key: request.access_key.trim().to_string(),
            secret_key,
        };
        crate::services::library::object_storage::LibraryObjectStorage::from_s3(&config)?
            .check()
            .await
    }

    pub async fn test_valkey_connection(
        &self,
        request: &crate::contracts::TestRuntimeValkeyRequest,
    ) -> Result<()> {
        let valkey_url = request.valkey_url.trim();
        if valkey_url.is_empty() {
            return Err(DomainError::invalid_argument(
                "runtime.scheduler.valkey_url must not be empty",
            )
            .into());
        }

        let client = redis::Client::open(valkey_url).map_err(|error| {
            DomainError::invalid_argument(format!("invalid runtime.scheduler.valkey_url: {error}"))
        })?;
        let mut connection = client.get_connection_manager().await.map_err(|error| {
            DomainError::internal(format!("failed to connect to Valkey: {error}"))
        })?;
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .map_err(|error| DomainError::internal(format!("Valkey PING failed: {error}")))?;
        Ok(())
    }

    pub async fn get_docling_settings(&self) -> Result<DoclingSettingsResponse> {
        let Some(settings) = self.db.get_docling_settings().await? else {
            return Ok(unconfigured_docling_response());
        };
        let has = self.docling_secrets.is_present().await?;
        Ok(response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings,
            has,
        ))
    }

    pub async fn update_docling_settings(
        &self,
        request: &UpdateDoclingSettingsRequest,
    ) -> Result<DoclingSettingsResponse> {
        settings_validate::docling_request(request)?;

        // An old client that omits the OCR preset keeps the stored selection,
        // so the save reads the stored row before it rebuilds it.
        let stored = self.db.get_docling_settings().await?;
        let ocr_preset = resolved_ocr_preset(
            request,
            stored
                .as_ref()
                .and_then(|settings| settings.ocr_engine.as_deref()),
        );

        let patch = docling_vlm_patch(
            request.vlm.api_key.clone(),
            normalize_optional_string(request.vlm.openai_base_url.clone()).is_some(),
        );
        let candidate = self
            .docling_secrets
            .stage(
                &patch,
                |api_key| docling_settings_from_request(request, api_key, &ocr_preset),
                validate_docling_vlm_shape,
            )
            .await?;

        let settings = self.db.save_docling_settings(&candidate).await?;
        if let Some(observer) = &self.docling_settings_observer {
            observer();
        }
        let has = self.docling_secrets.is_present().await?;
        Ok(response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings,
            has,
        ))
    }

    /// The Docling runtime config, with the API key resolved through the store.
    ///
    /// The value is handed to the provider in memory and is never logged, put in
    /// a task payload, or returned by an API.
    pub async fn resolve_docling_config(&self) -> Result<Option<DoclingConfig>> {
        let Some(settings) = self.db.get_docling_settings().await? else {
            return Ok(None);
        };
        let api_key = self.docling_secrets.resolve().await?;
        Ok(Some(config_from_stored(settings, api_key)))
    }

    pub async fn get_search_settings(&self) -> Result<SearchSettingsResponse> {
        let settings = self
            .db
            .get_search_settings()
            .await?
            .unwrap_or_else(default_search_settings);
        // Presence only: a settings projection never opens the stored key.
        let has = self.search_secrets.is_present().await?;
        Ok(search_response_from_stored(settings, has))
    }

    pub async fn update_search_settings(
        &self,
        request: &CanonicalUpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse> {
        settings_validate::canonical_search_request(request)?;

        let candidate = self
            .search_secrets
            .stage(
                &request.api_key,
                |api_key| canonical_search_settings_from_request(request, api_key),
                settings_validate::stored_search_settings,
            )
            .await?;

        let settings = self.db.save_search_settings(&candidate).await?;
        let has = self.search_secrets.is_present().await?;
        Ok(search_response_from_stored(settings, has))
    }

    /// Legacy wire-compat entry: validates and maps through the legacy
    /// wrappers (which delegate to the canonical kernel) without behavior
    /// change.
    pub async fn update_search_settings_legacy(
        &self,
        request: &UpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse> {
        settings_validate::search_request(request)?;

        let canonical = CanonicalUpdateSearchSettingsRequest::from(request.clone());
        let candidate = self
            .search_secrets
            .stage(
                &canonical.api_key,
                |api_key| search_settings_from_request(request, api_key),
                settings_validate::stored_search_settings,
            )
            .await?;

        let settings = self.db.save_search_settings(&candidate).await?;
        let has = self.search_secrets.is_present().await?;
        Ok(search_response_from_stored(settings, has))
    }
}

/// Applies one authoritative refresh under the shared save lock and the shared
/// runtime transition.
async fn refresh_runtime_configuration_under_lock(
    save_lock: &Mutex<()>,
    transition: Option<&RuntimeEmbeddingTransition>,
    refresher: Option<&RuntimeConfigRefresher>,
) -> Result<ConfigRefreshOutcome> {
    let Some(transition) = transition else {
        return Ok(ConfigRefreshOutcome::Deferred);
    };
    let Some(refresher) = refresher else {
        return Ok(ConfigRefreshOutcome::Deferred);
    };
    // The same lock a settings save and a confirmed acknowledgement hold. Taking
    // it before the transition means a refresh and a save are strictly ordered,
    // and the refresh always reads the state that won that order.
    let _save = save_lock.lock().await;
    if !transition.begin() {
        return Ok(ConfigRefreshOutcome::Deferred);
    }
    let _hold = TransitionHold(transition.clone());
    refresher().await
}

/// Applies authoritative refreshes on notification until shutdown.
///
/// A deferred refresh releases both locks and retries with a bounded backoff,
/// always re-reading the committed row, so it never replays a captured older
/// configuration and never holds up a save for the lifetime of a rebuild.
async fn reconcile_runtime_refreshes(
    requested: &AtomicBool,
    notify: &tokio::sync::Notify,
    shutdown: &tokio_util::sync::CancellationToken,
    mut refresh: impl FnMut() -> Pin<Box<dyn Future<Output = Result<ConfigRefreshOutcome>> + Send>>,
) {
    let mut backoff = Duration::from_millis(25);
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        if !requested.load(Ordering::Acquire) {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = notify.notified() => {}
            }
            continue;
        }
        // Cleared before the refresh, so a request that lands while it runs is
        // still pending and reloads the newer state afterwards.
        requested.store(false, Ordering::Release);
        match refresh().await {
            Ok(ConfigRefreshOutcome::Deferred) => {
                backoff = (backoff * 2).min(Duration::from_secs(2));
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(backoff) => {}
                }
                requested.store(true, Ordering::Release);
            }
            Ok(_) | Err(_) => backoff = Duration::from_millis(25),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mappers::{
            config_from_stored, docling_settings_from_request, resolved_ocr_preset,
            response_from_stored, search_response_from_stored, search_settings_from_request,
        },
        runtime_mappers::{runtime_settings_from_request, runtime_settings_response},
        unconfigured_docling_response,
        validate::{
            docling_request as validate_docling_request,
            runtime_settings_request as validate_runtime_settings_request,
            search_request as validate_search_request,
            stored_search_settings as validate_stored_search_settings,
        },
        validate_docling_vlm_shape,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use tokio::sync::Mutex;

    use super::{
        ConfigRefreshOutcome, RuntimeConfigRefresher, RuntimeEmbeddingTransition,
        reconcile_runtime_refreshes, refresh_runtime_configuration_under_lock,
    };
    use crate::{
        contracts::{
            DoclingSettingsSource, RuntimeChunkingSettings, RuntimeQdrantSettings,
            RuntimeSchedulerSettings, UpdateDoclingConnectionSettings,
            UpdateDoclingSettingsRequest, UpdateDoclingVlmSettings, UpdateRuntimeEmbeddingSettings,
            UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
        },
        db::{StoredDoclingSettings, default_search_settings},
    };

    fn sample_request() -> UpdateDoclingSettingsRequest {
        UpdateDoclingSettingsRequest {
            connection: UpdateDoclingConnectionSettings {
                base_url: "http://docling:5001".to_string(),
                timeout_secs: 120,
                poll_interval_secs: 2,
                task_timeout_secs: 3600,
                max_inflight: context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
            },
            vlm: UpdateDoclingVlmSettings::default(),
            ocr_preset: None,
        }
    }

    fn sample_stored() -> StoredDoclingSettings {
        StoredDoclingSettings {
            base_url: "http://docling:5001".to_string(),
            timeout_secs: 120,
            poll_interval_secs: 2,
            task_timeout_secs: 3600,
            max_inflight: context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
            pdf_backend: None,
            images_scale: None,
            image_export_mode: None,
            do_ocr: true,
            force_ocr: false,
            ocr_engine: None,
            ocr_lang: Vec::new(),
            do_code_enrichment: false,
            do_formula_enrichment: false,
            do_picture_description: false,
            openai_base_url: None,
            api_key: None,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
            picture_description_preset: None,
        }
    }

    fn sample_runtime_request() -> UpdateRuntimeSettingsRequest {
        UpdateRuntimeSettingsRequest {
            qdrant: RuntimeQdrantSettings {
                url: "http://qdrant:6334".to_string(),
                collection_name: "context69".to_string(),
                recreate_on_dimension_mismatch: false,
            },
            embedding: UpdateRuntimeEmbeddingSettings {
                base_url: "https://openrouter.ai/api/v1".to_string(),
                model: "text-embedding-3-large".to_string(),
                dimensions: 3072,
                timeout_secs: 30,
                api_key: None,
                confirm_vector_space_unchanged: false,
            },
            scheduler: RuntimeSchedulerSettings {
                interval_secs: 300,
                run_on_start: true,
                max_concurrency: 1,
                job_id: "context69-sync".to_string(),
                valkey_url: Some("redis://valkey:6379/0".to_string()),
            },
            chunking: RuntimeChunkingSettings {
                max_chars: 1200,
                overlap_chars: 200,
            },
            file_library: crate::contracts::UpdateRuntimeFileLibrarySettings {
                storage_root: "/tmp/library".to_string(),
                max_upload_size_mb: 128,
                max_upload_request_size_mb: 128,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
        }
    }

    #[test]
    fn docling_response_hides_api_key() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());

        let response = response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings.clone(),
            settings.api_key.is_some(),
        );
        assert_eq!(
            response.vlm.openai_base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(response.vlm.has_api_key);
        // Presence is an input, not a fact about the value, so a response can
        // report a key it is deliberately not holding open.
        assert!(
            !response_from_stored(DoclingSettingsSource::Database, true, settings, false)
                .vlm
                .has_api_key
        );
    }

    #[test]
    fn request_allows_docling_vlm_to_be_disabled() {
        validate_docling_request(&sample_request()).expect("request without vlm should be valid");
    }

    #[test]
    fn request_rejects_zero_docling_task_timeout() {
        let mut request = sample_request();
        request.connection.task_timeout_secs = 0;

        let error =
            validate_docling_request(&request).expect_err("zero task timeout should be rejected");
        assert!(error.to_string().contains("task_timeout_secs"));
    }

    #[test]
    fn stored_docling_settings_allow_vlm_to_be_disabled() {
        let settings = sample_stored();
        validate_docling_vlm_shape(&settings).expect("disabled vlm should be valid");
    }

    #[test]
    fn stored_docling_settings_accept_complete_raw_vlm() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        validate_docling_vlm_shape(&settings).expect("raw vlm settings should be valid");
    }

    #[test]
    fn stored_docling_settings_require_complete_raw_auth_fields() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("partial raw auth should be invalid");
        assert!(error.to_string().contains("openai_base_url"));
        assert!(error.to_string().contains("api_key"));
    }

    #[test]
    fn stored_docling_settings_require_auth_when_models_are_present() {
        let mut settings = sample_stored();
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("models without auth should fail");
        assert!(error.to_string().contains("openai_base_url"));
    }

    #[test]
    fn stored_docling_settings_require_models_when_auth_is_present() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("auth without models should fail");
        assert!(
            error
                .to_string()
                .contains("required when Docling VLM is configured")
        );
    }

    #[test]
    fn runtime_request_rejects_invalid_chunking() {
        let mut request = sample_runtime_request();
        request.chunking.overlap_chars = request.chunking.max_chars;

        let error =
            validate_runtime_settings_request(&request).expect_err("runtime request should fail");
        assert!(error.to_string().contains("overlap_chars"));
    }

    #[test]
    fn runtime_request_rejects_invalid_url_import_limits() {
        let mut request = sample_runtime_request();
        request.file_library.url_import_concurrency = 0;

        let error = validate_runtime_settings_request(&request)
            .expect_err("zero URL import workers should be rejected");
        assert!(error.to_string().contains("url_import_concurrency"));

        request.file_library.url_import_concurrency = 1;
        request.file_library.url_import_min_interval_ms = 0;
        let error = validate_runtime_settings_request(&request)
            .expect_err("zero URL import interval should be rejected");
        assert!(error.to_string().contains("url_import_min_interval_ms"));
    }

    #[test]
    fn trusted_proxy_setting_round_trips_and_defaults_off() {
        let mut request = sample_runtime_request();
        request.file_library.trusted_proxy_enabled = true;

        let stored = runtime_settings_from_request(&request);
        assert!(stored.file_library.trusted_proxy_enabled);
        assert!(
            runtime_settings_response(stored, false, false, false)
                .file_library
                .trusted_proxy_enabled
        );
        assert!(
            !crate::config::Config::default()
                .file_library
                .trusted_proxy_enabled
        );
    }

    #[test]
    fn the_s3_secret_key_is_never_persisted_and_is_reported_from_presence() {
        let mut request = sample_runtime_request();
        request.file_library.s3 = Some(crate::contracts::UpdateRuntimeS3Settings {
            endpoint: "https://objects.internal".to_string(),
            region: "internal".to_string(),
            bucket: "library".to_string(),
            prefix: "/staging/".to_string(),
            path_style: true,
            access_key: " AKIA ".to_string(),
            secret_key: None,
        });

        // The access key and the rest of the block are stored settings; the secret
        // key is not one of them, so the mapper leaves it unresolved and the store
        // remains its only representation.
        let stored = runtime_settings_from_request(&request);
        let s3 = stored
            .file_library
            .s3
            .clone()
            .expect("requested s3 settings are stored");
        assert_eq!(
            s3.secret_key, None,
            "the settings row must not carry the secret key"
        );
        assert_eq!(stored.embedding.api_key, None);
        assert_eq!(s3.access_key, "AKIA", "the access key is not a secret");
        assert_eq!(s3.prefix, "staging");

        // Presence is an input, not a fact about the value: a response can report
        // a sealed key it is deliberately not holding open.
        let response = runtime_settings_response(stored.clone(), false, true, false);
        let response_s3 = response.file_library.s3.expect("s3 settings are projected");
        assert!(response_s3.has_secret_key);
        assert_eq!(response_s3.access_key, "AKIA");
        let without_s3 = runtime_settings_response(stored, false, false, false);
        assert!(
            !without_s3
                .file_library
                .s3
                .expect("s3 settings are projected")
                .has_secret_key
        );
    }

    #[test]
    fn search_response_hides_api_key() {
        let settings = default_search_settings();

        // `has_api_key` is an input, never a fact about the stored value: the
        // response can report a sealed key it is deliberately not holding open.
        let response = search_response_from_stored(settings.clone(), true);
        assert!(response.has_api_key);
        assert!(!search_response_from_stored(settings, false).has_api_key);
    }

    fn sample_search_request() -> UpdateSearchSettingsRequest {
        UpdateSearchSettingsRequest {
            mode: crate::contracts::SearchMode::Hybrid,
            rerank_enabled: true,
            rerank_base_url: "https://openrouter.ai/api/v1".to_string(),
            rerank_model: "cohere/rerank-4-fast".to_string(),
            candidate_limit: 40,
            timeout_secs: 10,
            api_key: None,
            clear_api_key: false,
            vector_weight: context69_contracts::settings::SEARCH_VECTOR_WEIGHT_DEFAULT,
            keyword_weight: context69_contracts::settings::SEARCH_KEYWORD_WEIGHT_DEFAULT,
        }
    }

    #[test]
    fn search_request_rejects_empty_rerank_model() {
        let mut request = sample_search_request();
        request.rerank_model = " ".to_string();

        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("rerank_model"));
    }

    #[test]
    fn search_request_rejects_out_of_range_fusion_weights() {
        let mut request = sample_search_request();
        request.vector_weight = 1.5;
        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("vector_weight"));

        let mut request = sample_search_request();
        request.keyword_weight = -0.1;
        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("keyword_weight"));
    }

    #[test]
    fn search_request_rejects_weights_above_unit_budget() {
        let mut request = sample_search_request();
        request.vector_weight = 0.7;
        request.keyword_weight = 0.4;
        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("must not sum above 1"));
    }

    #[test]
    fn search_request_accepts_default_and_partial_fusion_weights() {
        validate_search_request(&sample_search_request()).expect("default weights are valid");
        let mut request = sample_search_request();
        request.vector_weight = 0.4;
        request.keyword_weight = 0.3;
        validate_search_request(&request).expect("weights under the unit budget are valid");
    }

    #[test]
    fn legacy_search_request_without_weights_defaults_to_current_blend() {
        let payload = serde_json::json!({
            "mode": "hybrid",
            "rerank_enabled": true,
            "rerank_base_url": "https://openrouter.ai/api/v1",
            "rerank_model": "cohere/rerank-4-fast",
            "candidate_limit": 40,
            "timeout_secs": 10
        });
        let request: UpdateSearchSettingsRequest =
            serde_json::from_value(payload).expect("legacy request without fusion weights");
        assert_eq!(request.vector_weight, 0.55);
        assert_eq!(request.keyword_weight, 0.35);
        validate_search_request(&request).expect("defaulted weights should be valid");
    }

    #[test]
    fn search_fusion_weights_round_trip_through_mappers_and_stored_validation() {
        let mut request = sample_search_request();
        request.vector_weight = 0.6;
        request.keyword_weight = 0.2;

        let stored = search_settings_from_request(&request, None);
        assert_eq!(stored.vector_weight, 0.6);
        assert_eq!(stored.keyword_weight, 0.2);
        validate_stored_search_settings(&stored).expect("stored weights should be valid");

        let response = search_response_from_stored(stored.clone(), false);
        assert_eq!(response.vector_weight, 0.6);
        assert_eq!(response.keyword_weight, 0.2);

        let mut invalid = stored;
        invalid.keyword_weight = 0.5;
        let error =
            validate_stored_search_settings(&invalid).expect_err("stored weights should fail");
        assert!(error.to_string().contains("sum above 1"));
    }

    #[test]
    fn picture_description_preset_round_trips_through_request_and_stored_config() {
        let mut request = sample_request();
        request.vlm = UpdateDoclingVlmSettings {
            picture_description_preset: Some("smolvlm".to_string()),
            ..UpdateDoclingVlmSettings::default()
        };

        let stored = docling_settings_from_request(&request, None, "rapidocr");
        assert_eq!(
            stored.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "settings mapper must persist picture_description_preset from request to stored"
        );

        let config = config_from_stored(stored.clone(), None);
        assert_eq!(
            config.vlm.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "stored -> runtime config must surface picture_description_preset"
        );

        let response = response_from_stored(DoclingSettingsSource::Database, true, stored, false);
        assert_eq!(
            response.vlm.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "response must include picture_description_preset without affecting has_api_key"
        );
        assert!(
            !response.vlm.has_api_key,
            "preset-only settings must not pretend to have an api key"
        );
    }

    #[test]
    fn empty_picture_description_preset_is_normalized_away_in_mapper() {
        let request = sample_request();
        let stored = docling_settings_from_request(&request, None, "rapidocr");
        assert!(
            stored.picture_description_preset.is_none(),
            "blank picture_description_preset must be normalized away by the mapper"
        );
    }

    #[test]
    fn validate_docling_vlm_shape_accepts_preset_only_configuration() {
        let mut settings = sample_stored();
        settings.picture_description_preset = Some("granite_vision".to_string());

        validate_docling_vlm_shape(&settings)
            .expect("preset-only VLM settings should be a valid stored configuration");
    }

    #[test]
    fn validate_docling_vlm_shape_rejects_preset_combined_with_partial_legacy_fields() {
        let mut settings = sample_stored();
        settings.picture_description_preset = Some("granite_vision".to_string());
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        // api_key deliberately missing to make the stored config unsafe
        // and exercise the explicit preset+legacy error message.

        let error = validate_docling_vlm_shape(&settings)
            .expect_err("preset combined with partial legacy fields must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("picture_description_preset"),
            "error must name picture_description_preset but was: {message}"
        );
        assert!(
            message.contains("legacy OpenAI VLM bundle"),
            "error must call out the legacy bundle but was: {message}"
        );
    }

    #[test]
    fn validate_docling_vlm_shape_rejects_preset_combined_with_complete_legacy_bundle() {
        let mut settings = sample_stored();
        settings.picture_description_preset = Some("granite_vision".to_string());
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let error = validate_docling_vlm_shape(&settings)
            .expect_err("preset combined with the full legacy bundle must be rejected");
        assert!(error.to_string().contains("picture_description_preset"));
    }

    #[test]
    fn validate_docling_vlm_shape_keeps_complete_custom_vlm_valid() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        validate_docling_vlm_shape(&settings)
            .expect("complete custom VLM bundle must remain valid");
    }

    #[test]
    fn validate_docling_vlm_shape_rejects_partial_custom_vlm() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        // picture_description_model + code_formula_model missing

        let error = validate_docling_vlm_shape(&settings)
            .expect_err("partial custom VLM bundle must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("fully configured together"),
            "error must call out the all-or-none model rule but was: {message}"
        );
    }

    #[test]
    fn docling_request_accepts_bounded_max_inflight() {
        let mut request = sample_request();
        for limit in [
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
        ] {
            request.connection.max_inflight = limit;
            validate_docling_request(&request).expect("bounded max_inflight should be valid");
        }
    }

    #[test]
    fn docling_request_rejects_out_of_range_max_inflight() {
        let mut request = sample_request();
        request.connection.max_inflight = 0;
        let error =
            validate_docling_request(&request).expect_err("zero max_inflight must be rejected");
        assert!(error.to_string().contains("max_inflight"));

        request.connection.max_inflight =
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX + 1;
        let error = validate_docling_request(&request)
            .expect_err("over-ceiling max_inflight must be rejected");
        assert!(error.to_string().contains("max_inflight"));
    }

    #[test]
    fn docling_mapper_round_trips_max_inflight() {
        let mut request = sample_request();
        request.connection.max_inflight = 3;
        let stored = docling_settings_from_request(&request, None, "rapidocr");
        assert_eq!(stored.max_inflight, 3);

        let response =
            response_from_stored(DoclingSettingsSource::Database, true, stored.clone(), false);
        assert_eq!(response.connection.max_inflight, 3);

        let config = config_from_stored(stored, None);
        assert_eq!(config.connection.max_inflight, 3);
    }

    #[test]
    fn legacy_docling_request_without_max_inflight_defaults_to_single_worker() {
        let payload = serde_json::json!({
            "connection": {
                "base_url": "http://docling:5001",
                "timeout_secs": 120,
                "poll_interval_secs": 2,
                "task_timeout_secs": 3600
            },
            "vlm": {}
        });
        let request: UpdateDoclingSettingsRequest =
            serde_json::from_value(payload).expect("legacy request without max_inflight");
        assert_eq!(
            request.connection.max_inflight,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
            "legacy API payloads must default to the single-worker ceiling"
        );
        validate_docling_request(&request).expect("defaulted request should be valid");
    }

    /// A legacy row without a stored preset reports the default preset, so a
    /// fresh or unconfigured installation shows one selection.
    #[test]
    fn an_unconfigured_docling_response_reports_the_default_ocr_preset() {
        let response = unconfigured_docling_response();
        assert_eq!(
            response.ocr_preset,
            context69_contracts::settings::DOCLING_OCR_PRESET_DEFAULT
        );

        let mut stored = sample_stored();
        stored.ocr_engine = None;
        let response = response_from_stored(DoclingSettingsSource::Database, true, stored, false);
        assert_eq!(
            response.ocr_preset,
            context69_contracts::settings::DOCLING_OCR_PRESET_DEFAULT
        );
    }

    /// A stored legacy value is a real selection and is never reset: `auto` is
    /// a preset of its own.
    #[test]
    fn a_stored_legacy_ocr_preset_is_preserved() {
        let mut stored = sample_stored();
        stored.ocr_engine = Some("auto".to_string());
        let response = response_from_stored(DoclingSettingsSource::Database, true, stored, false);
        assert_eq!(response.ocr_preset, "auto");

        let mut stored = sample_stored();
        stored.ocr_engine = Some("tesseract".to_string());
        let config = config_from_stored(stored, None);
        assert_eq!(config.ocr_preset, "tesseract");
    }

    /// A request that omits the preset keeps the stored selection, so an old
    /// client can never reset a saved choice.
    #[test]
    fn an_omitted_ocr_preset_keeps_the_stored_selection() {
        let request = sample_request();
        assert_eq!(request.ocr_preset, None);
        assert_eq!(
            resolved_ocr_preset(&request, Some("tesseract")),
            "tesseract",
            "an old client must not reset a saved preset"
        );
        assert_eq!(
            resolved_ocr_preset(&request, None),
            context69_contracts::settings::DOCLING_OCR_PRESET_DEFAULT,
            "an unset stored preset falls back to the default"
        );
    }

    /// An explicit selection wins and is stored as it is.
    #[test]
    fn a_selected_ocr_preset_is_persisted() {
        let mut request = sample_request();
        request.ocr_preset = Some(Some("custom.ocr-preset".to_string()));
        validate_docling_request(&request).expect("a valid preset must be accepted");

        let stored = docling_settings_from_request(&request, None, "custom.ocr-preset");
        assert_eq!(stored.ocr_engine.as_deref(), Some("custom.ocr-preset"));
        assert_eq!(
            resolved_ocr_preset(&request, Some("tesseract")),
            "custom.ocr-preset"
        );
    }

    /// An explicit null is rejected instead of being read as an absent field.
    ///
    /// The generated contract documents the property as a plain optional
    /// string, so `null` is not a value the API accepts.
    #[test]
    fn an_explicit_null_ocr_preset_is_rejected() {
        let mut request = sample_request();
        request.ocr_preset = Some(None);
        let error = validate_docling_request(&request)
            .expect_err("an explicit null preset must be rejected");
        let message = error.to_string();
        assert!(message.contains("ocr_preset"), "{message}");
        assert!(message.contains("must not be null"), "{message}");
    }

    /// An invalid preset is rejected instead of being coerced.
    #[test]
    fn an_invalid_ocr_preset_is_rejected() {
        for preset in ["", "   ", "rapid ocr", "rapid/ocr", "rápidocr"] {
            let mut request = sample_request();
            request.ocr_preset = Some(Some(preset.to_string()));
            let error =
                validate_docling_request(&request).expect_err("an invalid preset must be rejected");
            assert!(
                error.to_string().contains("ocr_preset"),
                "the error must name the rejected field, got: {error}"
            );
        }

        let mut request = sample_request();
        request.ocr_preset = Some(Some(
            "a".repeat(context69_contracts::settings::DOCLING_OCR_PRESET_MAX_LEN + 1),
        ));
        let error =
            validate_docling_request(&request).expect_err("an overlong preset must be rejected");
        assert!(error.to_string().contains("ocr_preset"), "{error}");

        let mut request = sample_request();
        request.ocr_preset = Some(Some(
            "a".repeat(context69_contracts::settings::DOCLING_OCR_PRESET_MAX_LEN),
        ));
        validate_docling_request(&request).expect("the longest accepted preset must be valid");
    }

    #[test]
    fn an_embedding_update_defaults_to_no_vector_space_confirmation() {
        let payload = serde_json::json!({
            "base_url": "https://gateway.example/v1",
            "model": "alias-model",
            "dimensions": 8,
            "timeout_secs": 30
        });
        let update: UpdateRuntimeEmbeddingSettings =
            serde_json::from_value(payload).expect("legacy embedding update");
        assert!(
            !update.confirm_vector_space_unchanged,
            "an omitted confirmation must never authorize keeping existing vectors"
        );
    }

    #[test]
    fn the_confirmation_is_transient_and_never_stored() {
        let mut request = sample_runtime_request();
        request.embedding.confirm_vector_space_unchanged = true;

        let stored = runtime_settings_from_request(&request);
        assert_eq!(
            stored.embedding.model, request.embedding.model,
            "the confirmation flag is not part of the stored embedding block"
        );
        assert_eq!(stored.embedding.api_key, None);
        assert_eq!(
            crate::embedding::EmbeddingIdentity::from_update(&request.embedding).model,
            request.embedding.model,
            "the confirmation does not change the candidate identity"
        );
    }

    #[test]
    fn the_response_reports_requires_rebuild_independently_of_credentials() {
        let stored = runtime_settings_from_request(&sample_runtime_request());
        assert!(!runtime_settings_response(stored.clone(), true, false, false).requires_rebuild);
        assert!(
            runtime_settings_response(stored, true, false, true).requires_rebuild,
            "the flag is projected from the persisted identity check"
        );
    }

    fn runtime_transition(
        runtime: &crate::vector_runtime::VectorRuntime,
    ) -> RuntimeEmbeddingTransition {
        let begin = runtime.clone();
        let end = runtime.clone();
        RuntimeEmbeddingTransition::new(
            move || begin.try_begin_identity_transition(),
            move || end.end_identity_transition(),
        )
    }

    #[tokio::test]
    async fn a_refresh_cannot_cross_the_settings_save_lock() {
        let save = Arc::new(Mutex::new(()));
        let runtime =
            crate::vector_runtime::VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
        let transition = runtime_transition(&runtime);
        let applied = Arc::new(AtomicBool::new(false));
        let refresher: RuntimeConfigRefresher = Arc::new({
            let applied = applied.clone();
            move || {
                applied.store(true, Ordering::SeqCst);
                Box::pin(async { Ok(ConfigRefreshOutcome::Applied) })
            }
        });

        // A save holds the shared lock: the observer refresh must wait for it
        // rather than install a configuration the save is replacing.
        let guard = save.lock().await;
        let handle = tokio::spawn({
            let save = save.clone();
            let refresher = refresher.clone();
            async move {
                refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
                    .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!applied.load(Ordering::SeqCst));
        assert!(!handle.is_finished());
        drop(guard);

        let outcome = handle
            .await
            .expect("the refresh finishes")
            .expect("no error");
        assert_eq!(outcome, ConfigRefreshOutcome::Applied);
        assert!(applied.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn an_in_flight_refresh_excludes_a_save_until_it_completes() {
        let save = Arc::new(Mutex::new(()));
        let runtime =
            crate::vector_runtime::VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
        let transition = runtime_transition(&runtime);
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let refresher: RuntimeConfigRefresher = Arc::new({
            let started = started.clone();
            let release = release.clone();
            move || {
                let started = started.clone();
                let release = release.clone();
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(ConfigRefreshOutcome::Applied)
                })
            }
        });

        let handle = tokio::spawn({
            let save = save.clone();
            async move {
                refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
                    .await
            }
        });
        started.notified().await;
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                save.clone().lock_owned()
            )
            .await
            .is_err(),
            "a save cannot obtain the shared lock while a refresh applies"
        );
        release.notify_one();
        handle.await.expect("refresh joins").expect("no error");

        // Once the refresh completes, the save proceeds and observes its state.
        let guard = save.lock().await;
        drop(guard);
    }

    #[tokio::test]
    async fn a_deferred_refresh_never_replays_an_older_captured_configuration() {
        let save = Arc::new(Mutex::new(()));
        let runtime =
            crate::vector_runtime::VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
        let transition = runtime_transition(&runtime);
        let fence = runtime.begin_rebuild();
        let authoritative = Arc::new(std::sync::Mutex::new("A".to_string()));
        let applied = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let refresher: RuntimeConfigRefresher = Arc::new({
            let authoritative = authoritative.clone();
            let applied = applied.clone();
            move || {
                let value = authoritative.lock().expect("authoritative").clone();
                applied.lock().expect("applied").push(value);
                Box::pin(async { Ok(ConfigRefreshOutcome::Applied) })
            }
        });

        // The runtime is rebuilding: the refresh defers without applying the
        // older value it could have captured.
        let outcome =
            refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
                .await
                .expect("no error");
        assert_eq!(outcome, ConfigRefreshOutcome::Deferred);
        assert!(applied.lock().expect("applied").is_empty());

        // A newer save/acknowledgement commits B while the notification is still
        // queued. Retrying loads B, the authoritative state, not A.
        *authoritative.lock().expect("authoritative") = "B".to_string();
        runtime.fail_rebuild(fence);
        runtime.set_requires_rebuild(false);
        let outcome =
            refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
                .await
                .expect("no error");
        assert_eq!(outcome, ConfigRefreshOutcome::Applied);
        assert_eq!(
            applied.lock().expect("applied").as_slice(),
            ["B".to_string()],
            "a deferred notification reloads the newest committed configuration"
        );

        // A further queued notification still reads the newest state.
        refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
            .await
            .expect("no error");
        assert_eq!(
            applied.lock().expect("applied").as_slice(),
            ["B".to_string(), "B".to_string()]
        );
    }

    #[tokio::test]
    async fn a_deferred_refresh_retries_with_bounded_waiting_and_exits_on_shutdown() {
        let requested = Arc::new(AtomicBool::new(true));
        let notify = Arc::new(tokio::sync::Notify::new());
        let shutdown = tokio_util::sync::CancellationToken::new();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let loop_attempts = attempts.clone();
        let handle = tokio::spawn({
            let requested = requested.clone();
            let notify = notify.clone();
            let shutdown = shutdown.clone();
            async move {
                reconcile_runtime_refreshes(&requested, &notify, &shutdown, move || {
                    let attempt =
                        loop_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    Box::pin(async move {
                        if attempt == 1 {
                            Ok(ConfigRefreshOutcome::Deferred)
                        } else {
                            Ok(ConfigRefreshOutcome::Applied)
                        }
                    })
                })
                .await;
            }
        });

        for _ in 0..200 {
            if attempts.load(std::sync::atomic::Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            attempts.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "a deferred refresh is retried automatically"
        );

        shutdown.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("cancellation ends the loop")
            .expect("the loop does not panic");
    }

    /// A runtime with a published, fingerprinted pair, exactly as recovery
    /// publishes one, so the production admission API can admit a manual start.
    fn published_runtime() -> crate::vector_runtime::VectorRuntime {
        struct Provider;

        #[async_trait::async_trait]
        impl crate::embedding::EmbeddingProvider for Provider {
            async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
                Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
            }
        }

        let runtime =
            crate::vector_runtime::VectorRuntime::configured(Arc::new(AtomicBool::new(true)));
        let generation = runtime.generation();
        assert!(runtime.publish(
            generation,
            crate::vector_runtime::VectorSnapshot {
                embedding: Arc::new(Provider),
                identity: Some(crate::embedding::EmbeddingIdentity {
                    base_url: "https://embeddings.example/v1".to_string(),
                    model: "model-a".to_string(),
                    dimensions: 4,
                }),
                index: Arc::new(crate::qdrant_index::QdrantIndex::for_test(
                    "collection-a",
                    4
                )),
                collection: "collection-a".to_string(),
                generation,
            }
        ));
        runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));
        runtime
    }

    fn recording_refresher(
        applied: Arc<AtomicBool>,
        outcome: ConfigRefreshOutcome,
    ) -> RuntimeConfigRefresher {
        Arc::new(move || {
            applied.store(true, Ordering::SeqCst);
            Box::pin(async move { Ok(outcome) })
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_paused_before_its_reservation_cannot_take_a_manual_rebuilds_generation() {
        let save = Arc::new(Mutex::new(()));
        let runtime = published_runtime();
        let transition = runtime_transition(&runtime);
        let applied = Arc::new(AtomicBool::new(false));
        let refresher = recording_refresher(applied.clone(), ConfigRefreshOutcome::Applied);

        // Hold the refresh at the exact old check-then-CAS window: it has
        // observed nothing yet and owns no reservation.
        let (paused_tx, mut paused_rx) = tokio::sync::mpsc::unbounded_channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        runtime.set_admission_pause(move || {
            let _ = paused_tx.send(());
            let _ = release_rx.recv();
        });

        let handle = tokio::spawn({
            let save = save.clone();
            let transition = transition.clone();
            let refresher = refresher.clone();
            async move {
                refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
                    .await
            }
        });
        paused_rx.recv().await.expect("the refresh is paused");

        // The manual rebuild wins while the refresh has no authority. Under the
        // old ordering the refresh would have observed `rebuilding == false`
        // here and then reserved after the start released.
        let admitted = runtime
            .try_admit_rebuild()
            .expect("the manual rebuild wins the runtime");
        let _ = release_tx.send(());

        let outcome = handle
            .await
            .expect("the refresh joins")
            .expect("no refresh error");
        assert_eq!(
            outcome,
            ConfigRefreshOutcome::Deferred,
            "the reserve-then-inspect admission refuses the rebuild's generation"
        );
        assert!(
            !applied.load(Ordering::SeqCst),
            "no configuration was applied under the rebuild"
        );
        assert!(
            runtime.is_rebuilding(),
            "the manual rebuild still owns its fence"
        );
        assert_eq!(
            runtime.generation(),
            admitted.generation,
            "the refused refresh advanced no generation"
        );
        assert!(!runtime.is_ready());

        // The refused deferred notification left no stranded fence: the owned
        // attempt settles, and a later credential refresh applies without
        // advancing its generation.
        runtime.set_requires_rebuild(false);
        assert!(runtime.complete_rebuild_success(admitted.generation));
        assert!(runtime.is_ready());
        let settled_generation = runtime.generation();
        let credential = recording_refresher(
            Arc::new(AtomicBool::new(false)),
            ConfigRefreshOutcome::AppliedUnchanged,
        );
        let outcome =
            refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&credential))
                .await
                .expect("the credential refresh runs");
        assert_eq!(outcome, ConfigRefreshOutcome::AppliedUnchanged);
        assert_eq!(
            runtime.generation(),
            settled_generation,
            "a credential-only refresh never advances the settled generation"
        );
        assert!(runtime.is_ready(), "the pair stays ready");
        assert!(!runtime.is_rebuilding());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_manual_start_is_refused_while_a_refresh_owns_the_reservation() {
        let save = Arc::new(Mutex::new(()));
        let runtime = published_runtime();
        let transition = runtime_transition(&runtime);

        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let refresher: RuntimeConfigRefresher = Arc::new({
            let started = started.clone();
            let release = release.clone();
            move || {
                let started = started.clone();
                let release = release.clone();
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(ConfigRefreshOutcome::Applied)
                })
            }
        });

        let handle = tokio::spawn({
            let save = save.clone();
            let transition = transition.clone();
            let refresher = refresher.clone();
            async move {
                refresh_runtime_configuration_under_lock(&save, Some(&transition), Some(&refresher))
                    .await
            }
        });
        started.notified().await;
        assert_eq!(
            runtime.try_admit_rebuild().err(),
            Some(crate::vector_runtime::RebuildAdmissionError::Busy),
            "the manual start cannot interleave with a held reservation"
        );
        assert!(!runtime.is_rebuilding());

        release.notify_one();
        let outcome = handle
            .await
            .expect("the refresh joins")
            .expect("no refresh error");
        assert_eq!(outcome, ConfigRefreshOutcome::Applied);
        assert!(!runtime.is_rebuilding());
    }

    #[test]
    fn the_shared_transition_refuses_a_reservation_while_a_rebuild_owns_the_runtime() {
        let runtime = published_runtime();
        let admitted = runtime.try_admit_rebuild().expect("the fence opens");
        assert!(
            !runtime.try_begin_identity_transition(),
            "reserve-then-inspect refuses while the fence is open"
        );
        assert!(
            !runtime.is_transitioning(),
            "the refused reservation releases the exclusion instead of holding it"
        );
        runtime.set_requires_rebuild(false);
        assert!(runtime.complete_rebuild_success(admitted.generation));
        assert!(
            runtime.try_begin_identity_transition(),
            "the same reservation succeeds once the rebuild settles"
        );
        runtime.end_identity_transition();
    }
}
