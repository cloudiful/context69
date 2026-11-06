//! Search registry epoch and registry entry storage (issue #1138 P3, lifecycle
//! semantics corrected by P9).
//!
//! The epoch row and the registry entries are the coherence foundation for the
//! future safe reingest design: searches snapshot the state before their data
//! reads and re-check the epoch after them. No production writer exists in
//! this phase; the transactional bump/register primitives below exist so the
//! tests exercise exactly the statement pair a future writer must combine,
//! and no registry mutation happens outside that transactional shape.
//!
//! The lifecycle owns the id arrays of an entry: a switch releases the staged
//! ids as the live version and stores the old live ids as the retired set, an
//! abandoned run turns its staged ids into the retired set, and a retired
//! entry keeps that set. The retired set is the tombstone of the ids that are
//! no longer live, so a resolved entry is never removed by the lifecycle.
//!
//! The state statement also reads the deleted entity sets (issue #1138 P24),
//! so one search attempt never pairs an epoch with a different set of deleted
//! files or deleted documents.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{Database, RegistryQuotaOverride, RegistryQuotaPolicy, VectorIndexState};
use context69_search::{RegistryPhase, SearchRegistryEntry, SearchRegistryState};

/// The persistent state of the rebuild barrier.
///
/// The barrier is the durable door between a full vector rebuild and every
/// registry-admitted publication. `Idle` is the only state that admits new
/// work; `Running` and `Failed` both fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildBarrierState {
    Idle,
    Running,
    Failed,
}

impl RebuildBarrierState {
    /// The stored state, or `None` for a state the barrier does not define.
    pub fn parse(state: &str) -> Option<Self> {
        match state {
            "idle" => Some(Self::Idle),
            "running" => Some(Self::Running),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// The barrier row of the registry epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildBarrier {
    pub state: RebuildBarrierState,
    /// The task item that owns a running or failed rebuild.
    pub owner_item_id: Option<Uuid>,
    /// The fencing token of the owner's captured grant.
    pub owner_token: Option<Uuid>,
    /// The identity the owner was admitted with.
    pub fingerprint: Option<String>,
    pub collection: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// A pause was requested and the worker has not reached its next batch
    /// boundary yet. The flag only exists while the barrier is running.
    pub pause_requested: bool,
    /// The last successfully written page, or `None` when no page committed.
    pub cursor: Option<RebuildCursor>,
    /// Committed chunks of this rebuild.
    pub processed_chunks: i64,
    /// Total chunks captured when the rebuild started.
    pub total_chunks: i64,
    /// The recorded target collection was created by the owner of this
    /// barrier.
    ///
    /// The marker is the durable proof of ownership: a recorded target with
    /// the marker set is a collection this attempt created and may be
    /// attached, while a recorded target without it is absent or ambiguous
    /// and is never adopted.
    pub rebuild_target_created: bool,
}

/// The committed position of a durable rebuild cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildCursor {
    pub document_id: i64,
    pub chunk_index: i32,
}

/// The committed progress one successful rebuild page records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildCheckpoint {
    pub cursor: RebuildCursor,
    pub processed_chunks: i64,
    pub total_chunks: i64,
}

impl RebuildBarrier {
    /// Whether `item` still owns this running barrier with `token`.
    pub fn is_owned_by(&self, item: Uuid, token: Uuid) -> bool {
        self.state == RebuildBarrierState::Running
            && self.owner_item_id == Some(item)
            && self.owner_token == Some(token)
    }
}

/// The vector identity a registry entry was admitted under.
///
/// An entry that admits new vector writes records the fingerprint of the
/// embedding space and the collection it was admitted with. The switch
/// compares the pair with the persisted configuration, so a node whose own
/// snapshot predates a settings change on another node cannot make its points
/// visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryVectorIdentity {
    pub embedding_fingerprint: String,
    pub collection_name: String,
}

/// Why a rebuild could not take the barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildRefusalReason {
    /// An unresolved staging entry blocks the rebuild.
    StagingEntries,
    /// Another rebuild owns the barrier and its owner is still a live writer.
    Busy,
    /// The barrier belongs to another vector identity.
    IdentityMismatch,
    /// The persisted configuration no longer describes the captured identity.
    PersistedIdentityMismatch,
    /// The persisted configuration has no vector identity at all.
    PersistedIdentityMissing,
}

/// The outcome of a rebuild start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildStartOutcome {
    /// The barrier is now owned by the caller.
    Started(RebuildBarrier),
    /// The barrier was not taken.
    Refused(RebuildRefusalReason),
}

/// The target choice one claim may make.
///
/// The unchecked primitive always writes the caller's collection. The checked
/// deployment entry lets the claim decide fresh versus recorded resume from the
/// barrier row it reads while it holds the epoch lock.
#[derive(Debug, Clone, Copy)]
enum RebuildTargetChoice<'a> {
    Given(&'a str),
    DecideFresh(&'a str),
}

/// The physical target a claim writes into and whether it resumes it.
///
/// This runs while the caller holds the epoch lock, so the decision is taken
/// from the barrier row the claim is about to update instead of a stale read
/// made before the lock.
fn rebuild_target_under_claim(
    state: RebuildBarrierState,
    recorded_fingerprint: Option<&str>,
    recorded_collection: Option<&str>,
    expected_fingerprint: &str,
    fresh_collection: &str,
) -> (String, bool) {
    if state == RebuildBarrierState::Idle {
        return (fresh_collection.to_string(), false);
    }
    match (recorded_fingerprint, recorded_collection) {
        (Some(fingerprint), Some(collection)) if fingerprint == expected_fingerprint => {
            (collection.to_string(), true)
        }
        _ => (fresh_collection.to_string(), false),
    }
}

/// The outcome of one registry admission.
///
/// The refusal reasons stay apart because they mean different things to the
/// caller: the caps are a budget decision that a later run may still fail,
/// while a busy barrier is a shared-resource wait that a retry resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionOutcome {
    Admitted(i64),
    CapsRefused,
    RebuildBusy,
}

/// The lifecycle edges the registry allows: a successful switch, the abandon
/// of a staging run, and the retirement of a switched run. Every other move is
/// refused before a statement runs, so an illegal transition can never mutate
/// a row.
fn validate_transition(expected_phase: &str, next_phase: &str) -> Result<()> {
    let legal = matches!(
        (expected_phase, next_phase),
        ("staging", "switched") | ("staging", "retired") | ("switched", "retired")
    );
    if !legal {
        bail!(
            "search registry transition {expected_phase} -> {next_phase} is not a lifecycle edge"
        );
    }
    Ok(())
}

fn validate_phase(label: &str, phase: &str) -> Result<()> {
    if RegistryPhase::parse(phase).is_some() {
        Ok(())
    } else {
        bail!("search registry {label} phase {phase:?} is not a lifecycle phase")
    }
}

#[derive(Debug, sqlx::FromRow)]
struct SearchRegistryStateRow {
    epoch: i64,
    entries: serde_json::Value,
    deleted_file_ids: Vec<Uuid>,
    deleted_document_ids: Vec<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct SearchRegistryEntryJson {
    run_id: Uuid,
    file_id: Option<Uuid>,
    document_ids: Vec<i64>,
    staged_ids: Vec<Uuid>,
    retired_ids: Vec<Uuid>,
    baseline_record_hash: Option<String>,
    staged_record_hash: Option<String>,
    generation: i64,
    phase: String,
}

fn parse_entries(value: serde_json::Value) -> Result<Vec<SearchRegistryEntry>> {
    let raw = serde_json::from_value::<Vec<SearchRegistryEntryJson>>(value)
        .context("decode search registry entries")?;
    raw.into_iter()
        .map(|entry| {
            validate_phase("stored", &entry.phase)?;
            Ok(SearchRegistryEntry {
                run_id: entry.run_id,
                file_id: entry.file_id,
                document_ids: entry.document_ids,
                staged_ids: entry.staged_ids,
                retired_ids: entry.retired_ids,
                baseline_record_hash: entry.baseline_record_hash,
                staged_record_hash: entry.staged_record_hash,
                generation: entry.generation,
                phase: entry.phase,
            })
        })
        .collect()
}

#[derive(sqlx::FromRow)]
struct RegistryEntryRow {
    run_id: Uuid,
    file_id: Option<Uuid>,
    document_ids: Vec<i64>,
    staged_ids: Vec<Uuid>,
    retired_ids: Vec<Uuid>,
    baseline_record_hash: Option<String>,
    staged_record_hash: Option<String>,
    generation: i64,
    phase: String,
}

#[derive(sqlx::FromRow)]
struct RebuildBarrierRow {
    rebuild_state: String,
    rebuild_owner_item: Option<Uuid>,
    rebuild_owner_token: Option<Uuid>,
    rebuild_fingerprint: Option<String>,
    rebuild_collection: Option<String>,
    rebuild_started_at: Option<DateTime<Utc>>,
    rebuild_error: Option<String>,
    rebuild_pause_requested: bool,
    rebuild_cursor_document_id: Option<i64>,
    rebuild_cursor_chunk_index: Option<i32>,
    rebuild_processed_chunks: i64,
    rebuild_total_chunks: i64,
    rebuild_target_created: bool,
}

#[derive(sqlx::FromRow)]
struct RegistryVectorIdentityRow {
    embedding_fingerprint: Option<String>,
    collection_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PersistedVectorIdentityRow {
    qdrant_url: String,
    collection_name: String,
    embedding_base_url: String,
    embedding_model: String,
    dimensions: i64,
}

impl Database {
    pub async fn read_search_registry_entry_in(
        &self,
        connection: &mut PgConnection,
        run_id: Uuid,
    ) -> Result<Option<SearchRegistryEntry>> {
        let row = sqlx::query_file_as!(
            RegistryEntryRow,
            "src/sql/db/search_registry/read_entry_for_update.sql",
            run_id
        )
        .fetch_optional(connection)
        .await?;
        row.map(|row| {
            validate_phase("stored", &row.phase)?;
            Ok(SearchRegistryEntry {
                run_id: row.run_id,
                file_id: row.file_id,
                document_ids: row.document_ids,
                staged_ids: row.staged_ids,
                retired_ids: row.retired_ids,
                baseline_record_hash: row.baseline_record_hash,
                staged_record_hash: row.staged_record_hash,
                generation: row.generation,
                phase: row.phase,
            })
        })
        .transpose()
    }

    pub async fn lock_search_registry_epoch_in(&self, connection: &mut PgConnection) -> Result<()> {
        sqlx::query_file_scalar!("src/sql/db/search_registry/lock_epoch.sql")
            .fetch_one(connection)
            .await
            .context("lock search registry epoch")?;
        Ok(())
    }

    /// Admit one staging entry under the registry epoch lock.
    ///
    /// This is the shape a writer that creates new vector points uses. The
    /// identity it recorded is the vector space of the node that staged the
    /// points, and the switch compares it with the persisted configuration
    /// before the points become visible. An entry that creates no point (a
    /// metadata-only tombstone) passes no identity.
    ///
    /// The admission refuses while a rebuild owns the barrier: a rebuild
    /// rewrites the whole collection, so a publication that overlaps it would
    /// be overwritten or lost. The refusal is reported apart from a caps
    /// refusal, because only one of them is a shared-resource wait.
    ///
    /// The caps that apply are the ones the deployment persisted at this
    /// moment: they are read inside this transaction, under the epoch lock.
    /// `quota` may only narrow them, so a caller can never admit more than the
    /// current policy allows.
    pub async fn admit_search_registry_entry_checked_in(
        &self,
        entry: &SearchRegistryEntry,
        identity: Option<&RegistryVectorIdentity>,
        anticipated_retired_count: i64,
        quota: RegistryQuotaOverride,
        connection: &mut PgConnection,
    ) -> Result<AdmissionOutcome> {
        let Some(file_id) = entry.file_id else {
            bail!("registry admission requires a file");
        };
        if entry.phase != "staging" || entry.staged_ids.is_empty() || !entry.retired_ids.is_empty()
        {
            bail!("registry admission requires a nonempty staging set and no retired ids");
        }
        if entry
            .staged_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            != entry.staged_ids.len()
        {
            bail!("registry admission has duplicate staged ids");
        }
        quota.validate()?;
        if anticipated_retired_count < 0 {
            bail!("registry admission requires nonnegative anticipation");
        }
        let staged_count =
            i64::try_from(entry.staged_ids.len()).context("staged id count overflow")?;
        let reservation = staged_count
            .checked_add(anticipated_retired_count)
            .context("reservation overflow")?;
        self.lock_search_registry_epoch_in(connection).await?;
        let barrier = self.read_rebuild_barrier_in(connection).await?;
        if barrier.state != RebuildBarrierState::Idle {
            return Ok(AdmissionOutcome::RebuildBusy);
        }
        // An entry that creates points may only be admitted for the one active
        // vector index that still matches the desired configuration. A
        // metadata-only tombstone passes no identity and creates no point, so
        // it keeps its existing path.
        if let Some(identity) = identity {
            self.require_admissible_vector_identity_in(connection, identity)
                .await?;
        }
        // The persisted policy is the authority. It is read here, inside the
        // admission transaction, so a node that still holds an older policy
        // cannot admit more than the deployment currently allows.
        let persisted = self.read_usable_registry_quota_in(connection).await?;
        let [global_cap, per_file_cap, per_run_cap] = persisted.narrowed(&quota).caps();
        if global_cap <= 0 || per_file_cap <= 0 || per_run_cap <= 0 {
            bail!("registry admission requires positive caps");
        }
        let usage = sqlx::query_file!(
            "src/sql/db/search_registry/read_usage.sql",
            file_id,
            &entry.staged_ids
        )
        .fetch_one(&mut *connection)
        .await
        .context("read registry usage")?;
        let total = usage
            .total
            .checked_add(reservation)
            .context("global usage overflow")?;
        let file_total = usage
            .file_total
            .checked_add(reservation)
            .context("file usage overflow")?;
        if usage.overlaps
            || reservation > per_run_cap
            || total > global_cap
            || file_total > per_file_cap
        {
            return Ok(AdmissionOutcome::CapsRefused);
        }
        sqlx::query_file!(
            "src/sql/db/search_registry/admit_reserved_entry.sql",
            entry.run_id,
            file_id,
            &entry.document_ids,
            &entry.staged_ids,
            &entry.retired_ids,
            entry.baseline_record_hash.as_deref(),
            entry.staged_record_hash.as_deref(),
            entry.generation,
            &entry.phase,
            reservation,
            identity.map(|identity| identity.embedding_fingerprint.as_str()),
            identity.map(|identity| identity.collection_name.as_str()),
        )
        .execute(&mut *connection)
        .await
        .context("admit registry entry")?;
        Ok(AdmissionOutcome::Admitted(
            self.bump_search_registry_epoch_in(connection).await?,
        ))
    }

    /// Admit one staging entry and report the new epoch, or nothing.
    ///
    /// A refused admission (the caps or a busy rebuild barrier) leaves the
    /// registry, the budget counters and the epoch untouched.
    pub async fn admit_search_registry_entry_in(
        &self,
        entry: &SearchRegistryEntry,
        anticipated_retired_count: i64,
        quota: RegistryQuotaOverride,
        connection: &mut PgConnection,
    ) -> Result<Option<i64>> {
        match self
            .admit_search_registry_entry_checked_in(
                entry,
                None,
                anticipated_retired_count,
                quota,
                connection,
            )
            .await?
        {
            AdmissionOutcome::Admitted(epoch) => Ok(Some(epoch)),
            AdmissionOutcome::CapsRefused | AdmissionOutcome::RebuildBusy => Ok(None),
        }
    }

    /// The registry capacity policy the deployment persisted.
    ///
    /// The row is the authority: a deployment that has no runtime file-library
    /// row yet has no persisted policy and uses the documented defaults.
    pub async fn read_registry_quota(&self) -> Result<RegistryQuotaPolicy> {
        let mut connection = self.pool.acquire().await?;
        self.read_registry_quota_in(&mut connection).await
    }

    /// The registry capacity policy, read on the caller's connection.
    pub async fn read_registry_quota_in(
        &self,
        connection: &mut PgConnection,
    ) -> Result<RegistryQuotaPolicy> {
        let row = sqlx::query_file!("src/sql/db/search_registry/read_registry_quota.sql")
            .fetch_optional(&mut *connection)
            .await
            .context("read the registry capacity policy")?;
        let Some(row) = row else {
            return Ok(RegistryQuotaPolicy::DEFAULT);
        };
        Ok(RegistryQuotaPolicy {
            max_ids_total: row.registry_max_ids_total,
            max_ids_per_file: row.registry_max_ids_per_file,
            max_ids_per_run: row.registry_max_ids_per_run,
        })
    }

    /// The registry capacity policy, refused when it is not usable.
    ///
    /// An invalid persisted policy is never replaced by a default: a broken
    /// row has to fail clearly instead of silently widening or disabling the
    /// registry.
    pub async fn read_usable_registry_quota_in(
        &self,
        connection: &mut PgConnection,
    ) -> Result<RegistryQuotaPolicy> {
        let policy = self.read_registry_quota_in(connection).await?;
        policy.validate()?;
        Ok(policy)
    }

    /// The persistent rebuild barrier of the registry epoch row.
    pub async fn read_rebuild_barrier(&self) -> Result<RebuildBarrier> {
        let mut connection = self.pool.acquire().await?;
        self.read_rebuild_barrier_in(&mut connection).await
    }

    /// The number of unresolved staging entries of the registry.
    ///
    /// An admitted run that never resolved still holds staged points, so it
    /// blocks a move of the vector space: the staged points belong to the
    /// space the collection was built with.
    pub async fn read_staging_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/search_registry/read_staging_count.sql")
                .fetch_one(&self.pool)
                .await
                .context("read the unresolved staging entries")?,
        )
    }

    /// The persistent rebuild barrier, read on the caller's connection.
    ///
    /// The caller must already hold the epoch row lock when the answer
    /// decides a write, so the barrier and the epoch describe one instant.
    pub async fn read_rebuild_barrier_in(
        &self,
        connection: &mut PgConnection,
    ) -> Result<RebuildBarrier> {
        let row = sqlx::query_file_as!(
            RebuildBarrierRow,
            "src/sql/db/search_registry/read_barrier.sql"
        )
        .fetch_one(&mut *connection)
        .await
        .context("read the rebuild barrier")?;
        let state = RebuildBarrierState::parse(&row.rebuild_state).ok_or_else(|| {
            anyhow::anyhow!(
                "the rebuild barrier is in the undefined state {:?}",
                row.rebuild_state
            )
        })?;
        let cursor = match (
            row.rebuild_cursor_document_id,
            row.rebuild_cursor_chunk_index,
        ) {
            (Some(document_id), Some(chunk_index)) => Some(RebuildCursor {
                document_id,
                chunk_index,
            }),
            (None, None) => None,
            _ => {
                return Err(anyhow::anyhow!(
                    "the rebuild barrier records an unpaired cursor"
                ));
            }
        };
        Ok(RebuildBarrier {
            state,
            owner_item_id: row.rebuild_owner_item,
            owner_token: row.rebuild_owner_token,
            fingerprint: row.rebuild_fingerprint,
            collection: row.rebuild_collection,
            started_at: row.rebuild_started_at,
            error: row.rebuild_error,
            pause_requested: row.rebuild_pause_requested,
            cursor,
            processed_chunks: row.rebuild_processed_chunks,
            total_chunks: row.rebuild_total_chunks,
            rebuild_target_created: row.rebuild_target_created,
        })
    }

    /// Take the rebuild barrier of one fenced owner.
    ///
    /// This is the barrier primitive: it owns the epoch serialization, the
    /// unresolved-staging refusal and the owner takeover rules, but it takes
    /// the captured identity on trust. A user-requested rebuild of the
    /// deployment must enter through [`Self::begin_rebuild_checked_in`], which
    /// verifies the captured identity against the persisted configuration
    /// under the same epoch lock before this claim.
    ///
    /// The epoch row is locked first, so the barrier, the unresolved staging
    /// entries and the ownership decision are one atomic decision.
    ///
    /// A rebuild may not start while an unresolved staging entry exists: that
    /// entry's points are not the live version yet, and a rebuild would race
    /// them. A failed barrier is taken over only by the same identity, and a
    /// running barrier only by a new owner whose predecessor is no longer a
    /// live writer. A different identity is never cleared automatically.
    pub async fn begin_rebuild_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        fingerprint: &str,
        collection: &str,
    ) -> Result<RebuildStartOutcome> {
        self.lock_search_registry_epoch_in(connection).await?;
        let (outcome, _) = self
            .claim_rebuild_barrier_in(
                connection,
                owner_item_id,
                owner_token,
                fingerprint,
                RebuildTargetChoice::Given(collection),
            )
            .await?;
        Ok(outcome)
    }

    /// Take the rebuild barrier only when the captured identity is still the
    /// persisted configuration.
    ///
    /// The persisted identity is read after the epoch row is held, so a
    /// settings save that commits a new model or destination either happens
    /// entirely before the comparison or entirely after the barrier claim.
    /// The captured identity is the vector space the caller would rebuild; it
    /// must still describe what the deployment persists, or the rebuild must
    /// stop and retry under the current configuration.
    ///
    /// Only the fingerprint is compared: it is the logical identity of the
    /// desired vector space, while `collection` is the physical target this
    /// rebuild writes into. A target is named after the identity it is built
    /// for, so its name never has to equal the logical collection the settings
    /// name.
    ///
    /// `fresh_collection` is the name a fresh claim would write. The fresh
    /// versus owned-resume decision is made here, under the epoch lock, from
    /// the barrier row the claim updates: an idle barrier always creates the
    /// fresh target, and only a running or failed barrier of the same
    /// fingerprint attaches its recorded target.
    pub async fn begin_rebuild_checked_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        fingerprint: &str,
        fresh_collection: &str,
    ) -> Result<(RebuildStartOutcome, bool)> {
        self.lock_search_registry_epoch_in(connection).await?;
        match self.read_persisted_vector_identity_in(connection).await? {
            Some(persisted) if persisted.embedding_fingerprint == fingerprint => {}
            Some(_) => {
                return Ok((
                    RebuildStartOutcome::Refused(RebuildRefusalReason::PersistedIdentityMismatch),
                    false,
                ));
            }
            None => {
                return Ok((
                    RebuildStartOutcome::Refused(RebuildRefusalReason::PersistedIdentityMissing),
                    false,
                ));
            }
        }
        self.claim_rebuild_barrier_in(
            connection,
            owner_item_id,
            owner_token,
            fingerprint,
            RebuildTargetChoice::DecideFresh(fresh_collection),
        )
        .await
    }

    /// The barrier claim of one rebuild start; the caller holds the epoch row.
    ///
    /// The second value reports whether the claim resumed the recorded target
    /// of the same identity. It is decided from the barrier row read under the
    /// caller's epoch lock and must be retained by the attempt across its
    /// network work.
    async fn claim_rebuild_barrier_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        fingerprint: &str,
        target: RebuildTargetChoice<'_>,
    ) -> Result<(RebuildStartOutcome, bool)> {
        let barrier = self.read_rebuild_barrier_in(connection).await?;
        let (collection, resumed) = match target {
            RebuildTargetChoice::Given(collection) => (collection.to_string(), false),
            RebuildTargetChoice::DecideFresh(fresh_collection) => rebuild_target_under_claim(
                barrier.state,
                barrier.fingerprint.as_deref(),
                barrier.collection.as_deref(),
                fingerprint,
                fresh_collection,
            ),
        };
        let staging_count =
            sqlx::query_file_scalar!("src/sql/db/search_registry/read_staging_count.sql")
                .fetch_one(&mut *connection)
                .await
                .context("read the unresolved staging entries")?;
        if staging_count > 0 {
            return Ok((
                RebuildStartOutcome::Refused(RebuildRefusalReason::StagingEntries),
                false,
            ));
        }
        // A paused rebuild is still the running rebuild: it must be resumed by
        // its owner, never taken over or restarted under it.
        if barrier.pause_requested {
            return Ok((
                RebuildStartOutcome::Refused(RebuildRefusalReason::Busy),
                false,
            ));
        }
        let identity_matches = barrier.fingerprint.as_deref() == Some(fingerprint)
            && barrier.collection.as_deref() == Some(collection.as_str());
        match barrier.state {
            RebuildBarrierState::Idle => {}
            RebuildBarrierState::Failed => {
                if !identity_matches {
                    return Ok((
                        RebuildStartOutcome::Refused(RebuildRefusalReason::IdentityMismatch),
                        false,
                    ));
                }
            }
            RebuildBarrierState::Running => {
                if !identity_matches {
                    return Ok((
                        RebuildStartOutcome::Refused(RebuildRefusalReason::IdentityMismatch),
                        false,
                    ));
                }
                let owner = barrier
                    .owner_item_id
                    .ok_or_else(|| anyhow::anyhow!("the running rebuild barrier has no owner"))?;
                let owner_token = barrier.owner_token.ok_or_else(|| {
                    anyhow::anyhow!("the running rebuild barrier has no owner token")
                })?;
                // The recorded owner is live only while its own grant is: a
                // reclaimed item carries a fresh token, and a terminal or
                // deleted item cannot write at all. An elapsed lease deadline
                // is not proof of death, so a live owner keeps the barrier.
                let live = sqlx::query_file_scalar!(
                    "src/sql/db/search_registry/read_owner_live.sql",
                    owner,
                    owner_token
                )
                .fetch_one(&mut *connection)
                .await
                .context("read the rebuild owner liveness")?;
                if live {
                    return Ok((
                        RebuildStartOutcome::Refused(RebuildRefusalReason::Busy),
                        false,
                    ));
                }
            }
        }
        sqlx::query_file_scalar!(
            "src/sql/db/search_registry/begin_rebuild.sql",
            owner_item_id,
            owner_token,
            fingerprint,
            &collection
        )
        .fetch_one(&mut *connection)
        .await
        .context("claim the rebuild barrier")?;
        Ok((
            RebuildStartOutcome::Started(self.read_rebuild_barrier_in(connection).await?),
            resumed,
        ))
    }

    /// Record a failed rebuild attempt.
    ///
    /// The owner and its identity stay recorded, so a retry of the same
    /// identity may take the barrier over again while a different identity is
    /// refused.
    pub async fn fail_rebuild_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        error: &str,
    ) -> Result<bool> {
        let updated = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/fail_rebuild.sql",
            owner_item_id,
            owner_token,
            error
        )
        .fetch_optional(&mut *connection)
        .await
        .context("record the failed rebuild")?;
        Ok(updated.is_some())
    }

    /// Complete an owned rebuild: the identity, the barrier and the epochs move
    /// together.
    ///
    /// The epoch row is locked before the identity row is written, so the
    /// canonical order is item, epoch, vector_index_state everywhere. The
    /// settings save takes the same order, so the two transactions serialize
    /// instead of deadlocking on each other.
    ///
    /// The verified target is recorded first and stays inactive, the previous
    /// active row is deactivated in one statement, and the exact target is
    /// activated in a second one. Two statements are required: the partial
    /// unique index that admits one active row is not deferrable, so a single
    /// unordered update could observe two live rows. The activation commit is
    /// the completion linearization point.
    ///
    /// A caller that does not own the barrier writes nothing: the identity is
    /// not persisted either, so a superseded attempt never describes a
    /// collection another owner rebuilt.
    pub async fn complete_owned_rebuild_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        state: &VectorIndexState<'_>,
    ) -> Result<Option<i64>> {
        self.lock_search_registry_epoch_in(connection).await?;
        // Ownership is checked under the epoch lock before any identity row is
        // written. A missing, stale or cancelled owner therefore writes
        // nothing: no identity, no epoch, no generation.
        let barrier = self.read_rebuild_barrier_in(connection).await?;
        if !barrier.is_owned_by(owner_item_id, owner_token) {
            return Ok(None);
        }
        self.save_vector_index_state_in(connection, state).await?;
        self.deactivate_active_vector_index_state_in(connection)
            .await?;
        self.activate_vector_index_state_in(connection, state.collection_name)
            .await?;
        let epoch = self
            .complete_rebuild_in(connection, owner_item_id, owner_token)
            .await?;
        if epoch.is_none() {
            return Ok(None);
        }
        self.bump_search_generation_in(connection).await?;
        Ok(epoch)
    }

    /// Records the committed cursor of one owned rebuild page.
    ///
    /// The owner, its grant, the captured fingerprint and the physical
    /// collection are all part of the predicate, so a superseded or cancelled
    /// attempt advances nothing.
    pub async fn advance_rebuild_cursor_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        fingerprint: &str,
        collection: &str,
        checkpoint: RebuildCheckpoint,
    ) -> Result<bool> {
        let updated = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/advance_rebuild_cursor.sql",
            owner_item_id,
            owner_token,
            fingerprint,
            collection,
            checkpoint.cursor.document_id,
            checkpoint.cursor.chunk_index,
            checkpoint.processed_chunks,
            checkpoint.total_chunks,
        )
        .fetch_optional(&mut *connection)
        .await
        .context("advance the rebuild cursor")?;
        Ok(updated.is_some())
    }

    /// Requests a pause of one owned running rebuild.
    ///
    /// The flag is durable and observed by the worker at its next batch
    /// boundary; the current page is never interrupted.
    pub async fn request_rebuild_pause_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
    ) -> Result<bool> {
        let requested = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/request_rebuild_pause.sql",
            owner_item_id,
            owner_token,
        )
        .fetch_optional(&mut *connection)
        .await
        .context("request the rebuild pause")?;
        Ok(requested.is_some())
    }

    /// Clears the pause request of the exact owned running rebuild.
    ///
    /// The caller has already requeued the parked item in the same
    /// transaction. A barrier that no longer belongs to that item matches no
    /// row, so a stale resume never changes a newer rebuild.
    pub async fn resume_rebuild_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
    ) -> Result<bool> {
        let resumed = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/resume_rebuild.sql",
            owner_item_id,
        )
        .fetch_optional(&mut *connection)
        .await
        .context("resume the rebuild")?;
        Ok(resumed.is_some())
    }

    /// Frees the rebuild barrier of a cancelled owner and bumps the epoch.
    ///
    /// `task_id` narrows the clear to the items of one cancelled task; `None`
    /// is the bulk cancellation, which frees every barrier whose recorded owner
    /// can no longer write. `only_failed` is the explicit discard of a terminal
    /// rebuild and never touches a running barrier.
    pub async fn cancel_owned_rebuild_in(
        &self,
        connection: &mut PgConnection,
        task_id: Option<Uuid>,
        only_failed: bool,
    ) -> Result<Option<i64>> {
        let epoch = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/cancel_owned_rebuild.sql",
            task_id,
            only_failed,
            Option::<Uuid>::None,
            Option::<Uuid>::None,
            Option::<&str>::None,
        )
        .fetch_optional(&mut *connection)
        .await
        .context("free the cancelled rebuild barrier")?;
        Ok(epoch)
    }

    /// Clears the exact owned barrier of a refused rebuild target.
    ///
    /// A target that this attempt may not adopt is never deleted or emptied:
    /// only the durable record is released, so the next explicit attempt
    /// derives a fresh target name instead of writing into a collection that
    /// is not provably its own.
    ///
    /// The clear is fenced by the owner item, its grant token and the
    /// recorded collection, so a superseded attempt frees its own barrier and
    /// never the barrier of a newer owner. The epoch row is locked first, so
    /// the clear and the ownership decision are one atomic decision.
    pub async fn clear_owned_rebuild_target_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        collection: &str,
    ) -> Result<Option<i64>> {
        self.lock_search_registry_epoch_in(connection).await?;
        let epoch = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/cancel_owned_rebuild.sql",
            Option::<Uuid>::None,
            false,
            owner_item_id,
            owner_token,
            collection,
        )
        .fetch_optional(&mut *connection)
        .await
        .context("clear the refused rebuild target")?;
        Ok(epoch)
    }

    /// Records that the owner created the recorded rebuild target.
    ///
    /// The marker is written under the exact owner: the owner item, its grant
    /// token, the captured fingerprint and the recorded collection are all
    /// part of the predicate, so a superseded or cancelled attempt marks
    /// nothing and no other collection is ever claimed.
    pub async fn mark_rebuild_target_created_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
        fingerprint: &str,
        collection: &str,
    ) -> Result<bool> {
        self.lock_search_registry_epoch_in(connection).await?;
        let epoch = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/mark_rebuild_target_created.sql",
            owner_item_id,
            owner_token,
            fingerprint,
            collection,
        )
        .fetch_optional(&mut *connection)
        .await
        .context("mark the created rebuild target")?;
        Ok(epoch.is_some())
    }

    /// Clear the rebuild barrier of its owner and publish the new epoch.
    ///
    /// Only the owner of the running barrier may clear it, so a superseded
    /// owner writes nothing at all.
    pub async fn complete_rebuild_in(
        &self,
        connection: &mut PgConnection,
        owner_item_id: Uuid,
        owner_token: Uuid,
    ) -> Result<Option<i64>> {
        let epoch = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/complete_rebuild.sql",
            owner_item_id,
            owner_token
        )
        .fetch_optional(&mut *connection)
        .await
        .context("clear the rebuild barrier")?;
        Ok(epoch)
    }

    /// Refuse a mutation while a rebuild owns the barrier.
    ///
    /// The epoch row is locked first, so the check and the mutation it
    /// guards are one atomic decision against every other registry writer.
    pub async fn require_rebuild_idle_in(&self, connection: &mut PgConnection) -> Result<()> {
        self.lock_search_registry_epoch_in(connection).await?;
        let barrier = self.read_rebuild_barrier_in(connection).await?;
        if barrier.state != RebuildBarrierState::Idle {
            return Err(crate::domain_errors::DomainError::conflict(
                "a vector index rebuild is in progress; retry after it completes",
            )
            .into());
        }
        Ok(())
    }

    /// The vector identity an entry was admitted under.
    pub async fn read_search_registry_entry_identity_in(
        &self,
        connection: &mut PgConnection,
        run_id: Uuid,
    ) -> Result<Option<RegistryVectorIdentity>> {
        let row = sqlx::query_file_as!(
            RegistryVectorIdentityRow,
            "src/sql/db/search_registry/read_entry_identity.sql",
            run_id
        )
        .fetch_optional(&mut *connection)
        .await
        .context("read the registry entry identity")?;
        Ok(row.and_then(
            |row| match (row.embedding_fingerprint, row.collection_name) {
                (Some(embedding_fingerprint), Some(collection_name)) => {
                    Some(RegistryVectorIdentity {
                        embedding_fingerprint,
                        collection_name,
                    })
                }
                _ => None,
            },
        ))
    }

    /// Refuse a switch whose entry was admitted under another vector identity.
    ///
    /// The active pointer and the desired configuration are the authority: they
    /// describe the vector space of the deployment, not what one node believes.
    /// A run that was admitted before a settings change on another node would
    /// publish points of the old space, so the switch refuses it and the caller
    /// retries under the current configuration.
    pub async fn require_matching_vector_identity_in(
        &self,
        connection: &mut PgConnection,
        run_id: Uuid,
    ) -> Result<()> {
        let entry_identity = self
            .read_search_registry_entry_identity_in(connection, run_id)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the registry entry records no vector identity; a run without an identity \
                     cannot publish points"
                )
            })?;
        self.require_admissible_vector_identity_in(connection, &entry_identity)
            .await
    }

    /// Refuse an ordinary point write unless the captured identity is the one
    /// active vector index that still matches the desired configuration.
    ///
    /// The caller must already hold the registry epoch row lock, so the desired
    /// settings, the active pointer and the captured identity describe one
    /// instant and no admission can slip between them. The desired fingerprint
    /// stays logical: a target built into a different physical collection is
    /// admissible as long as the active row records that same logical identity
    /// and the captured identity names the active physical collection.
    pub async fn require_admissible_vector_identity_in(
        &self,
        connection: &mut PgConnection,
        identity: &RegistryVectorIdentity,
    ) -> Result<()> {
        let desired = self
            .read_persisted_vector_identity_in(connection)
            .await?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::conflict(
                    "the deployment has no desired vector identity; configure the embedding and \
                     vector settings before writing points",
                )
            })?;
        let active = self
            .read_active_vector_index_state_in(connection)
            .await?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::conflict(
                    "the deployment has no active vector index; rebuild and activate a vector \
                     collection before writing points",
                )
            })?;
        if desired.embedding_fingerprint != active.fingerprint {
            return Err(crate::domain_errors::DomainError::conflict(
                "the desired vector configuration differs from the active vector index; rebuild \
                 and activate the configured vector space before writing points",
            )
            .into());
        }
        if identity.embedding_fingerprint != active.fingerprint
            || identity.collection_name != active.collection_name
        {
            return Err(crate::domain_errors::DomainError::conflict(
                "the captured vector identity does not match the active vector index; retry under \
                 the current configuration",
            )
            .into());
        }
        Ok(())
    }

    /// The persisted vector identity of the deployment.
    ///
    /// The configured identity is the authority: it is what the stored
    /// settings describe, not what one node believes.
    pub async fn read_persisted_vector_identity_in(
        &self,
        connection: &mut PgConnection,
    ) -> Result<Option<RegistryVectorIdentity>> {
        let row = sqlx::query_file_as!(
            PersistedVectorIdentityRow,
            "src/sql/db/search_registry/read_vector_identity.sql"
        )
        .fetch_optional(&mut *connection)
        .await
        .context("read the persisted vector identity")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let dimensions = usize::try_from(row.dimensions)
            .context("the configured embedding dimensions must be non-negative")?;
        Ok(Some(RegistryVectorIdentity {
            embedding_fingerprint: crate::embedding::embedding_runtime::vector_identity_fingerprint(
                row.qdrant_url.trim(),
                row.collection_name.trim(),
                row.embedding_base_url.trim(),
                row.embedding_model.trim(),
                dimensions,
            ),
            collection_name: row.collection_name.trim().to_string(),
        }))
    }

    /// The registry state (epoch plus entries plus the deleted entity sets)
    /// from one statement, so the snapshot can never pair an epoch with a
    /// different entry set or a different set of deleted files and documents.
    /// An empty registry is the normal answer.
    pub async fn read_search_registry_state(&self) -> Result<SearchRegistryState> {
        let row = sqlx::query_file_as!(
            SearchRegistryStateRow,
            "src/sql/db/search_registry/read_state.sql"
        )
        .fetch_one(&self.pool)
        .await
        .context("read search registry state")?;
        Ok(SearchRegistryState {
            epoch: row.epoch,
            entries: parse_entries(row.entries)?,
            deleted_file_ids: row.deleted_file_ids,
            deleted_document_ids: row.deleted_document_ids,
        })
    }

    /// The current registry epoch from one scalar read; the post-result
    /// coherence re-check of every search attempt.
    pub async fn read_search_registry_epoch(&self) -> Result<i64> {
        let epoch = sqlx::query_file_scalar!("src/sql/db/search_registry/read_epoch.sql")
            .fetch_one(&self.pool)
            .await
            .context("read search registry epoch")?;
        Ok(epoch)
    }

    /// Register one entry on a live transaction.
    ///
    /// A future writer must combine this statement with the epoch bump in one
    /// transaction; no production writer exists yet, so the only caller is the
    /// database test that pins that transactional shape.
    pub async fn register_search_registry_entry_in(
        &self,
        entry: &SearchRegistryEntry,
        connection: &mut PgConnection,
    ) -> Result<()> {
        validate_phase("entry", &entry.phase)?;
        sqlx::query_file!(
            "src/sql/db/search_registry/register_entry.sql",
            entry.run_id,
            entry.file_id,
            &entry.document_ids,
            &entry.staged_ids,
            &entry.retired_ids,
            entry.baseline_record_hash.as_deref(),
            entry.staged_record_hash.as_deref(),
            entry.generation,
            &entry.phase,
        )
        .execute(&mut *connection)
        .await
        .context("register search registry entry")?;
        Ok(())
    }

    /// Bump the registry epoch on a live transaction, returning the new value.
    ///
    /// Every registry mutation must bump the epoch in the same transaction;
    /// the caller owns the transaction so the bump commits with its entry
    /// change or rolls back with it.
    pub async fn bump_search_registry_epoch_in(
        &self,
        connection: &mut PgConnection,
    ) -> Result<i64> {
        let epoch = sqlx::query_file_scalar!("src/sql/db/search_registry/bump_epoch.sql")
            .fetch_one(&mut *connection)
            .await
            .context("bump search registry epoch")?;
        Ok(epoch)
    }

    /// Register one entry and bump the epoch in the caller's transaction.
    ///
    /// The pair is the committed shape of a new run: a duplicate `run_id`
    /// fails the insert before the bump, and the caller's rollback then leaves
    /// both the registry and the epoch untouched.
    pub async fn register_search_registry_entry_with_epoch_in(
        &self,
        entry: &SearchRegistryEntry,
        connection: &mut PgConnection,
    ) -> Result<i64> {
        self.lock_search_registry_epoch_in(connection).await?;
        self.register_search_registry_entry_in(entry, connection)
            .await?;
        self.bump_search_registry_epoch_in(connection).await
    }

    /// Atomically move one entry from `expected_phase` to `next_phase`.
    ///
    /// The `(run_id, expected_phase)` predicate is the compare-and-set: an
    /// entry that is absent or already past the expected phase matches no row,
    /// so the transition and its epoch bump are skipped and the caller gets
    /// `None`. A successful transition bumps the epoch exactly once, in the
    /// caller's transaction, so the phase change and the bump commit together.
    ///
    /// Only the three lifecycle edges are allowed, and every one of them owns
    /// the id arrays it leaves behind:
    ///
    /// * `staging -> switched` stores the complete caller-provided old-live
    ///   set as the retired ids and clears the staged ids, because the staged
    ///   ids are the live version from this point on and must not stay
    ///   excluded.
    /// * `staging -> retired` abandons the run: its stored staged ids become
    ///   the retired ids and the staged ids are cleared, so a late write of a
    ///   staged point stays excluded.
    /// * `switched -> retired` keeps the retired ids and clears the staged
    ///   ids, which are already empty.
    ///
    /// The caller therefore supplies the retired id set only for the switch;
    /// every other edge derives it from the stored arrays.
    pub async fn transition_search_registry_entry_in(
        &self,
        run_id: Uuid,
        expected_phase: &str,
        next_phase: &str,
        retired_ids: Option<&[Uuid]>,
        connection: &mut PgConnection,
    ) -> Result<Option<i64>> {
        validate_phase("expected", expected_phase)?;
        validate_phase("next", next_phase)?;
        validate_transition(expected_phase, next_phase)?;
        match (expected_phase, next_phase) {
            ("staging", "switched") => {
                if retired_ids.is_none() {
                    bail!("a switched search registry entry requires its retired id set");
                }
            }
            ("staging", "retired") => {
                if retired_ids.is_some() {
                    bail!(
                        "an abandoned search registry entry takes its retired ids from its staged ids"
                    );
                }
            }
            _ => {
                if retired_ids.is_some() {
                    bail!(
                        "a switched search registry entry keeps its stored retired ids when it retires"
                    );
                }
            }
        }
        self.lock_search_registry_epoch_in(connection).await?;
        let updated = sqlx::query_file_scalar!(
            "src/sql/db/search_registry/transition_phase.sql",
            run_id,
            expected_phase,
            next_phase,
            retired_ids
        )
        .fetch_optional(&mut *connection)
        .await
        .context("transition search registry entry phase")?;
        if updated.is_none() {
            return Ok(None);
        }
        let epoch = self.bump_search_registry_epoch_in(connection).await?;
        Ok(Some(epoch))
    }

    /// Atomically remove one entry that is still an empty staging row.
    ///
    /// The same compare-and-set rule as the transition: a stale or repeated
    /// removal matches no row, changes no data, and does not bump the epoch.
    /// A removal that succeeds bumps the epoch once in the caller's
    /// transaction.
    ///
    /// Only an empty staging row may be removed. A staging row that still
    /// holds ids belongs to a run that has not resolved, and a switched or
    /// retired row is the tombstone of an already resolved run: both keep
    /// their ids excluded, so removing them would leak a hidden point back
    /// into the search results.
    pub async fn delete_search_registry_entry_in(
        &self,
        run_id: Uuid,
        expected_phase: &str,
        connection: &mut PgConnection,
    ) -> Result<Option<i64>> {
        validate_phase("expected", expected_phase)?;
        if expected_phase != "staging" {
            bail!("a resolved search registry entry is a tombstone and is never removed");
        }
        self.lock_search_registry_epoch_in(connection).await?;
        let deleted =
            sqlx::query_file_scalar!("src/sql/db/search_registry/delete_entry.sql", run_id)
                .fetch_optional(&mut *connection)
                .await
                .context("delete search registry entry")?;
        if deleted.is_none() {
            return Ok(None);
        }
        let epoch = self.bump_search_registry_epoch_in(connection).await?;
        Ok(Some(epoch))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RebuildBarrierState, parse_entries, rebuild_target_under_claim, validate_phase,
        validate_transition,
    };

    fn entry_json(phase: &str) -> serde_json::Value {
        serde_json::json!([
            {
                "run_id": "00000000-0000-0000-0000-000000000000",
                "file_id": null,
                "document_ids": [],
                "staged_ids": [],
                "retired_ids": [],
                "baseline_record_hash": null,
                "staged_record_hash": null,
                "generation": 0,
                "phase": phase,
            }
        ])
    }

    #[test]
    fn only_an_owned_resume_attaches_the_recorded_target() {
        let (target, resumed) = rebuild_target_under_claim(
            RebuildBarrierState::Running,
            Some("fingerprint-a"),
            Some("recorded-target"),
            "fingerprint-a",
            "fresh-target",
        );
        assert_eq!(target, "recorded-target");
        assert!(resumed, "a same-identity running claim resumes its target");

        let (target, resumed) = rebuild_target_under_claim(
            RebuildBarrierState::Failed,
            Some("fingerprint-a"),
            Some("recorded-target"),
            "fingerprint-a",
            "fresh-target",
        );
        assert_eq!(target, "recorded-target");
        assert!(resumed, "a same-identity failed claim retries its target");
    }

    #[test]
    fn an_idle_claim_always_creates_a_fresh_target() {
        // The stale pre-read this decision replaces could have seen the old
        // owner just before the barrier returned to idle; the claim must not
        // attach that recorded target.
        let (target, resumed) = rebuild_target_under_claim(
            RebuildBarrierState::Idle,
            Some("fingerprint-a"),
            Some("cancelled-target"),
            "fingerprint-a",
            "fresh-target",
        );
        assert_eq!(target, "fresh-target");
        assert!(!resumed);
    }

    #[test]
    fn a_different_identity_never_reuses_the_recorded_target() {
        let (target, resumed) = rebuild_target_under_claim(
            RebuildBarrierState::Running,
            Some("fingerprint-a"),
            Some("recorded-target"),
            "fingerprint-b",
            "fresh-target",
        );
        assert_eq!(target, "fresh-target");
        assert!(!resumed);
    }

    #[test]
    fn a_stored_phase_the_lifecycle_does_not_define_is_refused() {
        let error = parse_entries(entry_json("staged"))
            .expect_err("an undefined stored phase must be refused");
        assert!(
            error.to_string().contains("not a lifecycle phase"),
            "the decode names the refused phase: {error}"
        );
        assert!(
            validate_phase("stored", "staged").is_err(),
            "an undefined phase is refused wherever it is checked"
        );
    }

    #[test]
    fn every_lifecycle_phase_decodes() {
        for phase in ["staging", "switched", "retired"] {
            let entries = parse_entries(entry_json(phase)).expect("a lifecycle phase decodes");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].phase, phase);
        }
    }

    #[test]
    fn only_the_three_lifecycle_edges_are_allowed() {
        for (expected_phase, next_phase) in [
            ("staging", "switched"),
            ("staging", "retired"),
            ("switched", "retired"),
        ] {
            assert!(
                validate_transition(expected_phase, next_phase).is_ok(),
                "the {expected_phase} -> {next_phase} edge is a lifecycle edge"
            );
        }
        for (expected_phase, next_phase) in [
            ("staging", "staging"),
            ("switched", "switched"),
            ("retired", "retired"),
            ("switched", "staging"),
            ("retired", "switched"),
            ("retired", "staging"),
        ] {
            assert!(
                validate_transition(expected_phase, next_phase).is_err(),
                "the {expected_phase} -> {next_phase} move is not a lifecycle edge"
            );
        }
    }
}
