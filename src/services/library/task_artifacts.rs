//! Typed task-item artifact storage (issue #975 P3).
//!
//! These helpers are internal and unused by the ordinary runtime until the
//! later phases migrate call sites. They own one narrow contract: an artifact's
//! bytes are staged through the existing group-scoped content-addressed backend,
//! read back and verified before any database handoff, and attached to a running
//! item only under its current lease. Inline payload bytes are removed in the
//! same statement that commits the reference; every failure leaves the inline
//! value in place and the provisional object reclaimable by the existing
//! reference-checked cleanup.
//!
//! The registry remains authoritative: no helper stores or trusts a duplicate
//! digest, size, group, backend or key outside
//! `context69.library_storage_objects`, and the task group is always joined from
//! the item's parent task.

use anyhow::Result;
use bytes::Bytes;
use tracing::warn;
use uuid::Uuid;

use crate::db::{
    AttachTaskItemArtifactRequest, TaskItemArtifactKind, TaskItemArtifactRef, TaskItemArtifactRow,
};
use crate::domain_errors::DomainError;
use crate::library_store::{TaskWriteDenied, TaskWriteFence};

use super::LibraryService;
use super::object_storage;
use super::streaming::{self, BoundedBytesSink};

/// Chunk size used while hashing an in-memory artifact body.
const STAGING_CHUNK_BYTES: usize = 128 * 1024;

/// Hard cap for one serialized or decoded task-item artifact.
///
/// The cap bounds a whole-document allocation; it is not a constant-space
/// claim. A larger artifact keeps the item's inline payload and is skipped with
/// an explicit warning instead of failing the pipeline.
pub(crate) const MAX_TASK_ITEM_ARTIFACT_BYTES: usize = 256 * 1024 * 1024;

/// A staged and readback-verified artifact object awaiting its reference.
///
/// Internal to the artifact helpers; no wire contract depends on it.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedTaskItemArtifact {
    pub object_id: Uuid,
    pub group_id: i64,
    pub sha256: String,
    pub size_bytes: i64,
}

impl LibraryService {
    /// Stage one artifact body and verify the stored bytes before returning.
    ///
    /// The body is written to a temporary staging key, hashed while it streams,
    /// finalized into the content-addressed key and registered with the usual
    /// 24-hour staging lease. The provisional object is read back and compared
    /// against its own digest and size; a mismatch releases it through the
    /// existing reference-checked staging rollback instead of leaving an
    /// unverified reference.
    #[doc(hidden)]
    pub async fn stage_task_item_artifact(
        &self,
        group_id: i64,
        content: Bytes,
        lease_token: Uuid,
    ) -> Result<StagedTaskItemArtifact> {
        let temp_key = object_storage::staging_object_key(group_id, Uuid::new_v4());
        let mut writer = self.storage.open_stream_writer(&temp_key).await?;
        let mut source = streaming::BytesChunkSource::new(content, STAGING_CHUNK_BYTES);
        let digest = match streaming::transfer(&mut source, &mut writer, usize::MAX).await {
            Ok(digest) => digest,
            Err(error) => {
                writer.abort(&temp_key).await;
                let _ = self.storage.delete(&temp_key).await;
                return Err(error);
            }
        };
        writer.close().await?;
        let object_id = match self
            .finalize_streamed_task_input(
                group_id,
                &temp_key,
                &digest.sha256,
                digest.size_bytes,
                lease_token,
            )
            .await
        {
            Ok(object_id) => object_id,
            Err(error) => {
                let _ = self.storage.delete(&temp_key).await;
                return Err(error);
            }
        };
        if let Err(error) = self.storage.delete(&temp_key).await {
            warn!(%error, temp_key, "failed to remove the artifact staging key");
        }

        let staged = StagedTaskItemArtifact {
            object_id,
            group_id,
            sha256: digest.sha256,
            size_bytes: digest.size_bytes,
        };
        if let Err(error) = self.verify_staged_artifact(&staged).await {
            self.discard_staged_artifact(&staged).await;
            return Err(error);
        }
        Ok(staged)
    }

    /// Active storage backend used by the maintenance backfill CAS.
    #[doc(hidden)]
    pub fn maintenance_storage_backend(&self) -> &str {
        self.storage.backend()
    }

    /// Stage one artifact for the terminal-item maintenance backfill.
    ///
    /// A terminal item has no lease; the random token only scopes the existing
    /// storage dependency gate. Verification and cleanup stay identical to the
    /// running-item path.
    #[doc(hidden)]
    pub async fn stage_maintenance_task_artifact(
        &self,
        group_id: i64,
        content: Bytes,
    ) -> Result<StagedTaskItemArtifact> {
        self.stage_task_item_artifact(group_id, content, Uuid::new_v4())
            .await
    }

    /// Return a maintenance-staged object to the reference-checked staging
    /// rollback after a refused attach.
    #[doc(hidden)]
    pub async fn discard_maintenance_task_artifact(&self, staged: &StagedTaskItemArtifact) {
        self.discard_staged_artifact(staged).await;
    }

    /// Drop the staging lease after a terminal reference committed.
    #[doc(hidden)]
    pub async fn promote_maintenance_task_artifact(
        &self,
        staged: &StagedTaskItemArtifact,
    ) -> Result<()> {
        self.store
            .promote_task_item_artifact_object(staged.object_id)
            .await?;
        Ok(())
    }

    /// Attach a verified artifact to a running item under its captured grant.
    ///
    /// The object is re-verified immediately before the database handoff, then
    /// one guarded transaction commits the reference, removes only that kind's
    /// inline bytes and drops the staging lease of the object. A refused grant
    /// rolls the whole transaction back, so the payload, the reference and the
    /// object lease are exactly what they were, and the provisional object is
    /// handed back to the existing staging rollback.
    #[doc(hidden)]
    pub async fn attach_task_item_artifact(
        &self,
        fence: &TaskWriteFence,
        kind: TaskItemArtifactKind,
        staged: &StagedTaskItemArtifact,
    ) -> Result<bool> {
        self.verify_staged_artifact(staged).await?;
        let mut write = match self.store.begin_task_write(*fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => {
                // The grant is gone: the attach is refused, not failed.
                self.discard_staged_artifact(staged).await;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let attached = self
            .db
            .attach_task_item_artifact_in(
                write.connection(),
                AttachTaskItemArtifactRequest {
                    item_id: fence.item_id(),
                    lease_token: fence.token(),
                    kind,
                    storage_object_id: staged.object_id,
                    sha256: &staged.sha256,
                    size_bytes: staged.size_bytes,
                    storage_backend: self.storage.backend(),
                },
            )
            .await?;
        if !attached {
            write.rollback().await;
            self.discard_staged_artifact(staged).await;
            return Ok(false);
        }
        self.store
            .promote_task_item_artifact_object_in(write.connection(), staged.object_id)
            .await?;
        if !write.commit().await? {
            self.discard_staged_artifact(staged).await;
            return Ok(false);
        }
        Ok(true)
    }

    /// Load and validate the artifact reference for an item.
    ///
    /// `None` means there is no reference row at all. An existing reference with
    /// a NULL task group, a registry group in another group, an inactive
    /// backend, an unknown kind/encoding pair or a missing registry identity is
    /// an error: treating it as absent would let a stage reconvert the item or
    /// fall back to inline bytes instead of reporting a broken checkpoint.
    #[doc(hidden)]
    pub async fn load_task_item_artifact(
        &self,
        item_id: Uuid,
        kind: TaskItemArtifactKind,
    ) -> Result<Option<TaskItemArtifactRef>> {
        // The only profile where the relation is known absent is the pinned
        // pending-upgrade recovery admission, which this build derives from the
        // migration ledger rather than from a query error. Any real schema
        // error on an ordinary database propagates below.
        let policy = self.task_item_artifact_schema_policy();
        if !policy.allows_lookup() {
            return Ok(None);
        }
        let row = classify_artifact_lookup(
            policy,
            self.db.get_task_item_artifact(item_id, kind.kind()).await,
        )?;
        row.map(|row| validate_artifact_reference(row, kind, self.storage.backend()))
            .transpose()
    }

    /// Read one artifact, fully verified, under a caller byte cap.
    ///
    /// The registry size is checked before any read so a wrong or hostile
    /// metadata row cannot allocate past the cap; the same cap is enforced again
    /// per chunk while the object streams, and bytes are returned only after the
    /// digest and size match the stored identity.
    #[doc(hidden)]
    pub async fn read_task_item_artifact(
        &self,
        item_id: Uuid,
        kind: TaskItemArtifactKind,
        max_bytes: usize,
    ) -> Result<Option<Bytes>> {
        let Some(reference) = self.load_task_item_artifact(item_id, kind).await? else {
            return Ok(None);
        };
        if reference.size_bytes < 0 || reference.size_bytes as u64 > max_bytes as u64 {
            return Err(DomainError::payload_too_large("artifact_too_large").into());
        }
        let Some(mut stream) = self.storage.open_read_stream(&reference.object_key).await? else {
            return Err(DomainError::not_found(format!(
                "task item artifact object {} is missing",
                reference.storage_object_id
            ))
            .into());
        };
        let mut sink = BoundedBytesSink::new(max_bytes);
        let digest = streaming::transfer(&mut stream, &mut sink, max_bytes).await?;
        if digest.size_bytes != reference.size_bytes || digest.sha256 != reference.sha256 {
            return Err(DomainError::conflict("task item artifact integrity check failed").into());
        }
        Ok(Some(sink.into_bytes()))
    }

    /// Read one stored artifact object back and compare it with the staged
    /// identity. Nothing is returned: a caller proceeds only when this succeeds.
    async fn verify_staged_artifact(&self, staged: &StagedTaskItemArtifact) -> Result<()> {
        let object = self
            .store
            .get_storage_object_by_id(staged.object_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found(format!("unknown artifact object {}", staged.object_id))
            })?;
        if object.group_id != staged.group_id
            || object.sha256 != staged.sha256
            || object.size_bytes != staged.size_bytes
        {
            return Err(DomainError::conflict(
                "artifact object identity no longer matches the staged bytes",
            )
            .into());
        }
        if object.storage_backend != self.storage.backend() {
            return Err(DomainError::conflict(format!(
                "artifact object uses inactive backend {}",
                object.storage_backend
            ))
            .into());
        }
        let Some(mut stream) = self.storage.open_read_stream(&object.object_key).await? else {
            return Err(DomainError::not_found(format!(
                "artifact object {} is missing",
                staged.object_id
            ))
            .into());
        };
        let cap = usize::try_from(staged.size_bytes).map_err(|_| {
            anyhow::Error::from(DomainError::payload_too_large("artifact_too_large"))
        })?;
        let mut sink = BoundedBytesSink::new(cap);
        let digest = streaming::transfer(&mut stream, &mut sink, cap).await?;
        if digest.size_bytes != staged.size_bytes || digest.sha256 != staged.sha256 {
            return Err(DomainError::conflict("artifact readback integrity check failed").into());
        }
        Ok(())
    }

    /// Return a provisional object to the existing staging cleanup.
    ///
    /// The reference-checked rollback only removes bytes that nothing else
    /// references; a deduplicated shared object survives, and an unreferenced
    /// one otherwise expires through the 24-hour staging sweep.
    pub(crate) async fn discard_staged_artifact(&self, staged: &StagedTaskItemArtifact) {
        if let Err(error) = self
            .release_task_input_staging(staged.object_id, None)
            .await
        {
            warn!(
                object_id = %staged.object_id,
                %error,
                "failed to release the unverified artifact object; the staging sweep will reclaim it"
            );
        }
    }

    /// Externalize one item's serialized sections after its inline save.
    ///
    /// The inline payload is already durable when this runs, so a crash before
    /// the attach leaves a normal inline checkpoint and a crash after it leaves
    /// a reference without inline bytes; neither path resubmits a remote job.
    /// A NULL task group and an oversized serialization keep the inline value
    /// and are reported instead of failing the pipeline. A lost lease or a
    /// storage failure returns the error with the inline value untouched.
    ///
    /// On success the item's in-memory snapshot no longer carries
    /// `section_payload`, so a later payload save in the same claim cannot
    /// reinsert the externalized text.
    #[doc(hidden)]
    pub async fn externalize_task_item_sections(
        &self,
        item: &mut crate::db::ClaimedItem,
        sections: &serde_json::Value,
    ) -> Result<()> {
        let Some(group_id) = item.group_id else {
            return Ok(());
        };
        // Keep the inline sections and skip staging entirely in the pinned
        // pending-upgrade recovery profile: the artifact relation does not
        // exist yet, so an upload could never be referenced.
        if !self.task_item_artifact_schema_policy().allows_lookup() {
            return Ok(());
        }
        let encoded = match serde_json::to_vec(sections) {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!(
                    item_id = %item.id,
                    %error,
                    "task item sections are not serializable; keeping the inline checkpoint"
                );
                return Ok(());
            }
        };
        if encoded.len() > MAX_TASK_ITEM_ARTIFACT_BYTES {
            warn!(
                item_id = %item.id,
                size_bytes = encoded.len(),
                limit_bytes = MAX_TASK_ITEM_ARTIFACT_BYTES,
                "task item sections exceed the artifact cap; keeping the inline checkpoint"
            );
            return Ok(());
        }
        let staged = self
            .stage_task_item_artifact(group_id, Bytes::from(encoded), item.lease_token)
            .await?;
        let fence = TaskWriteFence::from_claimed_item(item)?;
        let attached = self
            .attach_task_item_artifact(&fence, TaskItemArtifactKind::Sections, &staged)
            .await?;
        if !attached {
            return Err(DomainError::conflict(
                "task item lease was lost while externalizing sections",
            )
            .into());
        }
        if let Some(object) = item.payload.as_object_mut() {
            object.remove("section_payload");
        }
        Ok(())
    }
}

impl LibraryService {
    /// Schema policy for artifact access in this database profile.
    ///
    /// Only the pinned pending-upgrade admission knows the relation is absent
    /// because the ledger says the migration has not run. It is never inferred
    /// from a SQL error code: an `undefined_table` on any other profile is a
    /// real schema fault and must propagate.
    pub(crate) fn task_item_artifact_schema_policy(&self) -> ArtifactSchemaPolicy {
        ArtifactSchemaPolicy::from_pending_upgrade(self.db.pending_upgrade().is_some())
    }
}

/// Validate an existing artifact row before any object read.
///
/// The registry and the item's parent task are the only identity sources; every
/// refusal names the broken part so an operator sees a real defect instead of a
/// silent "no data".
fn validate_artifact_reference(
    row: TaskItemArtifactRow,
    kind: TaskItemArtifactKind,
    active_backend: &str,
) -> Result<TaskItemArtifactRef> {
    if TaskItemArtifactKind::from_parts(&row.kind, &row.encoding) != Some(kind) {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} uses unknown kind/encoding {}/{}",
            row.storage_object_id, row.item_id, row.kind, row.encoding
        ))
        .into());
    }
    let Some(task_group_id) = row.task_group_id else {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} belongs to a task without a group",
            row.storage_object_id, row.item_id
        ))
        .into());
    };
    let Some(object_group_id) = row.object_group_id else {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} has no registry object",
            row.storage_object_id, row.item_id
        ))
        .into());
    };
    if object_group_id != task_group_id {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} references an object in another group",
            row.storage_object_id, row.item_id
        ))
        .into());
    }
    let Some(storage_backend) = row.storage_backend else {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} has no registry backend",
            row.storage_object_id, row.item_id
        ))
        .into());
    };
    if storage_backend != active_backend {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} uses inactive backend {storage_backend}",
            row.storage_object_id, row.item_id
        ))
        .into());
    }
    let Some(sha256) = row.sha256 else {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} has no registry digest",
            row.storage_object_id, row.item_id
        ))
        .into());
    };
    let Some(size_bytes) = row.size_bytes else {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} has no registry size",
            row.storage_object_id, row.item_id
        ))
        .into());
    };
    let Some(object_key) = row.object_key else {
        return Err(DomainError::conflict(format!(
            "task item artifact {} for item {} has no registry key",
            row.storage_object_id, row.item_id
        ))
        .into());
    };
    Ok(TaskItemArtifactRef {
        item_id: row.item_id,
        kind: row.kind,
        storage_object_id: row.storage_object_id,
        group_id: object_group_id,
        sha256,
        size_bytes,
        object_key,
        storage_backend,
        encoding: row.encoding,
    })
}

/// Apply the schema policy to one artifact lookup result.
///
/// The policy never converts a query error into an absent reference, so a
/// missing or corrupt relation on an ordinary database surfaces unchanged.
fn classify_artifact_lookup(
    policy: ArtifactSchemaPolicy,
    result: Result<Option<TaskItemArtifactRow>>,
) -> Result<Option<TaskItemArtifactRow>> {
    match result {
        Ok(reference) => Ok(reference),
        Err(_) if policy.treats_lookup_error_as_absent() => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether artifact relation access is possible in the current database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactSchemaPolicy {
    /// Ordinary database: the relation exists (or a missing one is an error).
    Present,
    /// Pinned pending-upgrade recovery: the migration has not run yet.
    KnownAbsentBeforeRecovery,
}

impl ArtifactSchemaPolicy {
    /// Derive the policy from the actual admission result, never from an
    /// observed query failure.
    pub(crate) fn from_pending_upgrade(pending_upgrade: bool) -> Self {
        if pending_upgrade {
            Self::KnownAbsentBeforeRecovery
        } else {
            Self::Present
        }
    }

    /// Whether an artifact read/write may touch the relation at all.
    pub(crate) fn allows_lookup(self) -> bool {
        matches!(self, Self::Present)
    }

    /// Whether a lookup failure may be reported as "no reference".
    ///
    /// Always false: the recovery profile skips the lookup instead of
    /// translating errors, so `undefined_table` on an ordinary database is
    /// never swallowed as an absent reference.
    pub(crate) fn treats_lookup_error_as_absent(self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactSchemaPolicy, classify_artifact_lookup};

    #[test]
    fn an_ordinary_database_queries_the_relation_and_propagates_errors() {
        let policy = ArtifactSchemaPolicy::from_pending_upgrade(false);
        assert_eq!(policy, ArtifactSchemaPolicy::Present);
        assert!(policy.allows_lookup());
        assert!(
            !policy.treats_lookup_error_as_absent(),
            "a normal database must never turn undefined_table into an absent reference"
        );
    }

    #[test]
    fn only_the_pinned_recovery_admission_skips_the_relation() {
        let policy = ArtifactSchemaPolicy::from_pending_upgrade(true);
        assert_eq!(policy, ArtifactSchemaPolicy::KnownAbsentBeforeRecovery);
        assert!(
            !policy.allows_lookup(),
            "the recovery profile keeps inline data and skips artifact staging"
        );
        assert!(!policy.treats_lookup_error_as_absent());
    }

    /// The exact regression this policy exists for: an ordinary database must
    /// surface `undefined_table` instead of reporting "no artifact".
    #[tokio::test]
    async fn a_normal_database_propagates_undefined_table_from_the_relation() {
        let policy = ArtifactSchemaPolicy::from_pending_upgrade(false);
        let classified =
            classify_artifact_lookup(policy, Err(anyhow::anyhow!("undefined_table: 42P01")));
        let error = classified.expect_err("a normal lookup failure must propagate");
        assert!(
            error.to_string().contains("undefined_table"),
            "the original schema error is preserved, got: {error}"
        );
    }

    /// Only the pinned recovery profile skips the relation; it does so by not
    /// querying at all, which the caller enforces through `allows_lookup`.
    #[test]
    fn a_present_relation_never_treats_an_error_as_absent() {
        assert!(!ArtifactSchemaPolicy::Present.treats_lookup_error_as_absent());
    }
}
