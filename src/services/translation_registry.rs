//! Registry-visible translation publication (issue #1138 P20).
//!
//! A translation publisher that owns a database handle can make the translated
//! points, the translation version and the search-registry visibility change
//! one atomic commit. The shared translation crate knows nothing about the
//! registry; this module is the application adapter that composes the two.
//!
//! The ordering is fixed: the bounded staging admission commits before the
//! first point write, the points are written insert-only and verified, and the
//! commit takes the registry epoch lock before the document row, re-reads the
//! actually live translation ids under that lock, writes the version and its
//! chunks, and switches the staging entry to its exact retired set in one
//! transaction. A refused admission or a moved source document leaves no
//! version row and no live point change.

use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use context69_search::SearchRegistryEntry;
use context69_translation::{TranslationPublication, TranslationStore, TranslationVersionInput};
use uuid::Uuid;

use crate::chunk_payload::{
    ChunkRef, PayloadDocument, PayloadGroup, PayloadLocale, translated_chunk,
};
use crate::chunking::{ChunkBudget, chunk_document_iter_with_budget};
use crate::contracts::Visibility;
use crate::db::Database;
use crate::domain::{ChunkPayload, SourceRecord};
use crate::normalize::normalize_record;
use crate::qdrant_index::QdrantIndex;
use crate::qdrant_index::metadata_retirement::OriginalPointExpectation;
use crate::vector_runtime::VectorSnapshot;

/// One planned publication, prepared before any write.
struct PlannedTranslation {
    generation: i64,
    normalized_hash: String,
    payloads: Vec<ChunkPayload>,
}

/// The handles one registry-visible publication needs.
pub(crate) struct Publication<'a> {
    pub db: &'a Database,
    pub store: &'a TranslationStore,
    pub snapshot: &'a VectorSnapshot,
    pub chunking: &'a ChunkBudget,
    /// The narrowing of the persisted registry capacity policy this
    /// publication admits under.
    pub quota: crate::db::RegistryQuotaOverride,
    /// The vector identity the publication writes its points under.
    ///
    /// The admission records it and the switch compares it with the persisted
    /// configuration, so a node whose snapshot predates a settings change on
    /// another node cannot make its points visible.
    pub identity: Option<crate::db::RegistryVectorIdentity>,
}

/// Publishes one translated version through the search registry.
///
/// Returns `Ok(None)` when the publication is not eligible for the registry
/// path (an empty translation), so the caller can fall back to the legacy
/// sequence instead of failing.
pub(crate) async fn publish_translation(
    handles: &Publication<'_>,
    file_id: Uuid,
    old_chunk_ids: &[Uuid],
    translation: &TranslationPublication<'_>,
    version: &TranslationVersionInput<'_>,
) -> Result<Option<()>> {
    let Some(plan) = plan(handles, translation, old_chunk_ids).await? else {
        return Ok(None);
    };
    let staged_ids: Vec<Uuid> = plan
        .payloads
        .iter()
        .map(|payload| payload.chunk_id)
        .collect();

    admit(handles, file_id, version, plan.generation, &staged_ids).await?;

    if let Err(error) = write_points(handles.snapshot, &plan).await {
        abandon_staging(
            handles.db,
            version.id,
            &staged_ids,
            handles.snapshot.index.as_ref(),
        )
        .await;
        return Err(error);
    }

    match commit(handles, version, &plan).await {
        Ok(Outcome::Committed(retired)) => {
            delete_retired(handles.snapshot.index.as_ref(), &retired).await;
            Ok(Some(()))
        }
        Ok(Outcome::SourceMoved) => {
            abandon_staging(
                handles.db,
                version.id,
                &staged_ids,
                handles.snapshot.index.as_ref(),
            )
            .await;
            Err(crate::domain_errors::DomainError::conflict(
                "translation source document changed during publication",
            )
            .into())
        }
        Ok(Outcome::TransitionRefused) => {
            abandon_staging(
                handles.db,
                version.id,
                &staged_ids,
                handles.snapshot.index.as_ref(),
            )
            .await;
            Err(crate::domain_errors::DomainError::unavailable(
                "translation publication exceeded its registry reservation; retry later",
            )
            .into())
        }
        Err(error) => {
            // The commit outcome is unknown: the registry entry may already be
            // switched, so the staged ids may be live. Never abandon or delete
            // them here.
            Err(error)
        }
    }
}

/// Builds the generation-scoped publication plan without writing anything.
async fn plan(
    handles: &Publication<'_>,
    translation: &TranslationPublication<'_>,
    old_chunk_ids: &[Uuid],
) -> Result<Option<PlannedTranslation>> {
    let generation = {
        let mut connection = handles.db.pool().acquire().await?;
        handles
            .db
            .next_ingest_generation_in(&mut connection)
            .await?
    };
    let normalized = normalize_record(SourceRecord {
        external_id: translation.external_id.to_string(),
        title: translation.title.to_string(),
        summary: translation.summary.map(ToOwned::to_owned),
        body_text: translation.body_text.to_string(),
        source_uri: translation.source_uri.to_string(),
        published_at: translation.published_at,
        updated_at: translation.updated_at,
        metadata_json: translation.metadata_json.clone(),
    });
    let chunk_source_key = format!(
        "translation:{}:{}",
        translation.source_key, translation.target_locale
    );
    let chunks = chunk_document_iter_with_budget(
        translation.document_id,
        &chunk_source_key,
        &normalized,
        handles.chunking,
        Some(generation),
    )?
    .collect::<Result<Vec<_>, _>>()?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let old: BTreeSet<Uuid> = old_chunk_ids.iter().copied().collect();
    ensure!(
        chunks.iter().all(|chunk| !old.contains(&chunk.id)),
        "generation-scoped translation ids collide with the current live ids"
    );
    let visibility = translation
        .visibility
        .parse::<Visibility>()
        .map_err(|error| {
            crate::domain_errors::DomainError::invalid_argument(format!(
                "invalid translation document visibility: {error}"
            ))
        })?;
    let group = PayloadGroup {
        group_id: translation.group_id,
        group_key: translation.group_key,
        group_path: translation.group_path,
        visibility,
    };
    let document = PayloadDocument {
        source_key: translation.source_key,
        external_id: translation.external_id,
        title: translation.title,
        summary: translation.summary,
        source_uri: translation.source_uri,
        published_at: translation.published_at,
        updated_at_source: translation.updated_at,
        record_hash: &normalized.record_hash,
        metadata_json: translation.metadata_json,
    };
    let locale = PayloadLocale {
        content_locale: translation.target_locale,
        source_locale: translation.source_locale,
        translation_provider: Some(translation.provider_key),
    };
    let payloads = chunks
        .iter()
        .map(|chunk| {
            translated_chunk(
                &group,
                &document,
                ChunkRef {
                    chunk_id: chunk.id,
                    document_id: translation.document_id,
                    chunk_index: chunk.chunk_index,
                    chunk_text: &chunk.text,
                },
                locale,
            )
        })
        .collect::<Vec<_>>();
    Ok(Some(PlannedTranslation {
        generation,
        normalized_hash: normalized.record_hash.clone(),
        payloads,
    }))
}

/// Commits the bounded staging admission before any point write.
///
/// The reservation covers every stored point id of the target locale, the
/// current one and the stale ones, because the publication replaces all of
/// them. The set is read on the admission transaction under the registry
/// epoch, so the reservation and the committed registry agree.
async fn admit(
    handles: &Publication<'_>,
    file_id: Uuid,
    version: &TranslationVersionInput<'_>,
    generation: i64,
    staged_ids: &[Uuid],
) -> Result<()> {
    let identity = handles.identity.as_ref().ok_or_else(|| {
        crate::domain_errors::DomainError::unavailable(
            "the translation publication has no vector identity; the runtime must publish a \
             configured vector space before it writes points",
        )
    })?;
    let mut tx = handles.db.pool().begin().await?;
    handles.db.lock_search_registry_epoch_in(&mut tx).await?;
    // The captured identity must still describe the persisted configuration.
    // The persisted rows are the authority: a node whose pair predates a
    // settings change on another node must not stage points under the older
    // identity.
    if let Some(persisted) = handles
        .db
        .read_persisted_vector_identity_in(&mut tx)
        .await?
        && persisted != *identity
    {
        tx.rollback().await?;
        return Err(crate::domain_errors::DomainError::conflict(
            "the configured vector identity changed since the translation captured it; retry it \
             under the current configuration",
        )
        .into());
    }
    let stored_ids = handles
        .store
        .all_translation_chunk_ids_in(&mut tx, version.document_id, Some(version.target_locale))
        .await?;
    let anticipated_retired_count =
        i64::try_from(stored_ids.len()).context("anticipated retired count overflow")?;
    let entry = SearchRegistryEntry {
        run_id: version.id,
        file_id: Some(file_id),
        document_ids: vec![version.document_id],
        staged_ids: staged_ids.to_vec(),
        retired_ids: Vec::new(),
        baseline_record_hash: Some(version.source_record_hash.to_string()),
        staged_record_hash: Some(version.source_record_hash.to_string()),
        generation,
        phase: "staging".into(),
    };
    let admitted = handles
        .db
        .admit_search_registry_entry_checked_in(
            &entry,
            Some(identity),
            anticipated_retired_count,
            handles.quota,
            &mut tx,
        )
        .await?;
    match admitted {
        crate::db::AdmissionOutcome::Admitted(_) => {}
        crate::db::AdmissionOutcome::CapsRefused => {
            tx.rollback().await?;
            // The registry capacity is a policy of the deployment, not a
            // temporary condition: the refusal is deterministic.
            return Err(crate::domain_errors::DomainError::invalid_argument(
                "the registry capacity policy does not allow this translation publication; \
                 retire the retained registry entries or raise the capacity policy",
            )
            .into());
        }
        crate::db::AdmissionOutcome::RebuildBusy => {
            tx.rollback().await?;
            return Err(crate::domain_errors::DomainError::conflict(
                "a vector index rebuild is in progress; retry the translation after it completes",
            )
            .into());
        }
    }
    tx.commit()
        .await
        .context("commit the translation registry admission")?;
    Ok(())
}

/// Embeds and writes every staged point insert-only, then verifies identity.
async fn write_points(snapshot: &VectorSnapshot, plan: &PlannedTranslation) -> Result<()> {
    let texts: Vec<String> = plan
        .payloads
        .iter()
        .map(|payload| payload.chunk_text.clone())
        .collect();
    let embeddings = snapshot.embedding.embed_texts(&texts).await?;
    ensure!(
        embeddings.len() == plan.payloads.len(),
        "the embedding provider returned the wrong vector count"
    );
    snapshot
        .index
        .upsert_document_chunks_insert_only(&plan.payloads, &embeddings)
        .await?;
    let expectations = plan
        .payloads
        .iter()
        .map(|payload| OriginalPointExpectation {
            point_id: payload.chunk_id,
            document_id: payload.document_id,
            record_hash: plan.normalized_hash.clone(),
        })
        .collect::<Vec<_>>();
    snapshot.index.verify_original_points(&expectations).await
}

/// The outcome of the commit attempt.
enum Outcome {
    /// The version, the chunks, the switch and the generation bump committed.
    Committed(Vec<Uuid>),
    /// The source document moved under the commit lock.
    SourceMoved,
    /// The registry transition refused the actual retired set.
    TransitionRefused,
}

/// Commits the version, chunks and registry switch in one transaction.
async fn commit(
    handles: &Publication<'_>,
    version: &TranslationVersionInput<'_>,
    plan: &PlannedTranslation,
) -> Result<Outcome> {
    let mut tx = handles.db.pool().begin().await?;
    handles.db.lock_search_registry_epoch_in(&mut tx).await?;
    // A rebuild owns the whole collection while it runs, so a publication
    // that overlapped it would write points the rebuild overwrites or loses.
    handles.db.require_rebuild_idle_in(&mut tx).await?;
    // The identity the publication was admitted under is compared with the
    // persisted configuration inside the commit transaction. The persisted
    // settings are the authority, so a node whose snapshot predates a
    // settings change on another node cannot make its points visible.
    handles
        .db
        .require_matching_vector_identity_in(&mut tx, version.id)
        .await?;
    let current = handles
        .store
        .lock_document_hash_in(&mut tx, version.document_id)
        .await?;
    if current != version.source_record_hash {
        tx.rollback().await?;
        return Ok(Outcome::SourceMoved);
    }
    // The retired set is the complete set of stored point ids of the target
    // locale, the current one and the stale ones, minus the ids this
    // publication writes. It is read under the document lock, so it is exactly
    // the set the switch replaces.
    let actual_old = handles
        .store
        .all_translation_chunk_ids_in(&mut tx, version.document_id, Some(version.target_locale))
        .await?;
    let new_ids: BTreeSet<Uuid> = plan
        .payloads
        .iter()
        .map(|payload| payload.chunk_id)
        .collect();
    let retired: Vec<Uuid> = actual_old
        .iter()
        .copied()
        .filter(|id| !new_ids.contains(id))
        .collect();
    let chunks = plan
        .payloads
        .iter()
        .map(|payload| (payload.chunk_id, payload.chunk_text.clone()))
        .collect::<Vec<_>>();
    if !handles
        .store
        .publish_version_in(&mut tx, version, &chunks)
        .await?
    {
        tx.rollback().await?;
        return Ok(Outcome::SourceMoved);
    }
    if handles
        .db
        .transition_search_registry_entry_in(
            version.id,
            "staging",
            "switched",
            Some(&retired),
            &mut tx,
        )
        .await?
        .is_none()
    {
        tx.rollback().await?;
        return Ok(Outcome::TransitionRefused);
    }
    handles.db.bump_search_generation_in(&mut tx).await?;
    tx.commit()
        .await
        .context("commit the translation publication")?;
    Ok(Outcome::Committed(retired))
}

/// Abandons a staging entry and deletes only its own staged points.
///
/// A staging entry that already resolved is never abandoned or deleted: its
/// ids may be the live version.
async fn abandon_staging(db: &Database, run_id: Uuid, staged_ids: &[Uuid], index: &QdrantIndex) {
    let abandoned = match db.pool().begin().await {
        Ok(mut tx) => {
            match db
                .transition_search_registry_entry_in(run_id, "staging", "retired", None, &mut tx)
                .await
            {
                Ok(Some(_)) => match tx.commit().await {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(run_id = %run_id, error = %error, "failed to commit translation abandonment");
                        false
                    }
                },
                Ok(None) => {
                    let _ = tx.rollback().await;
                    false
                }
                Err(error) => {
                    let _ = tx.rollback().await;
                    tracing::warn!(run_id = %run_id, error = %error, "failed to abandon translation staging entry");
                    false
                }
            }
        }
        Err(error) => {
            tracing::warn!(run_id = %run_id, error = %error, "failed to open translation abandonment transaction");
            false
        }
    };
    if !abandoned || staged_ids.is_empty() {
        return;
    }
    if let Err(error) = index.delete_points(staged_ids).await {
        tracing::warn!(run_id = %run_id, error = %error, "failed to delete abandoned translation points");
    }
}

/// Deletes the exact retired points best effort.
///
/// The registry tombstone already keeps them excluded, so a failure is a
/// pending cleanup rather than a failed publication.
async fn delete_retired(index: &QdrantIndex, retired: &[Uuid]) {
    if retired.is_empty() {
        return;
    }
    if let Err(error) = index.delete_points(retired).await {
        tracing::warn!(error = %error, "failed to delete retired translation points; keeping them tombstoned");
    }
}
