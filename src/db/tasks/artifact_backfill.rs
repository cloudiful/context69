//! Metadata and terminal attach queries for the explicit artifact backfill
//! operator tool (issue #975 P5).
//!
//! The preview scans never read field text and never mutate state. The apply
//! path fetches exactly one field once, and the terminal attach is a single
//! fingerprinted statement that commits the reference and removes only the
//! migrated inline key.

use anyhow::Result;
use uuid::Uuid;

use crate::db::Database;

use super::TaskItemArtifactKind;

/// One terminal item/kind metadata row produced by a bounded preview scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBackfillRow {
    pub item_id: Uuid,
    pub kind: TaskItemArtifactKind,
    pub task_id: Uuid,
    pub group_id: Option<i64>,
    pub item_status: String,
    pub task_status: String,
    pub task_deleted: bool,
    pub lease_active: bool,
    pub field_type: Option<String>,
    pub encoded_len: Option<i64>,
    pub declared_sha256_present: bool,
    pub declared_sha256_valid: bool,
    pub artifact_present: bool,
}

/// One fetched field for apply verification. Callers must not log or serialize
/// `field_text` or `declared_sha256`.
#[derive(Debug, Clone)]
pub struct ArtifactBackfillField {
    pub item_id: Uuid,
    pub group_id: Option<i64>,
    pub item_status: String,
    pub task_status: String,
    pub task_deleted: bool,
    pub lease_active: bool,
    pub field_type: Option<String>,
    /// Present only when the fetch statement itself proved the field still
    /// eligible and under the caller's cap.
    pub field_text: Option<String>,
    pub declared_sha256: Option<String>,
    /// Why the projection withheld the text (`race_changed`, `oversize`,
    /// `no_inline`, `invalid_shape`), or `None`.
    pub skip_reason: Option<String>,
}

/// One existing artifact reference as seen by the inverse (restore) scan.
///
/// The expected registry identity is carried so the restore CAS can reject a
/// retarget that happened after the scan. Digest values are never logged or
/// serialized into the operator report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRestoreRow {
    pub item_id: Uuid,
    pub task_id: Uuid,
    pub kind: TaskItemArtifactKind,
    pub encoding: String,
    pub storage_object_id: Uuid,
    pub object_group_id: i64,
    pub object_sha256: String,
    pub object_size_bytes: i64,
    pub object_storage_backend: String,
    pub item_status: String,
    pub task_status: String,
    pub task_deleted: bool,
    pub lease_active: bool,
    pub task_group_id: Option<i64>,
    pub inline_type: Option<String>,
    pub download_metadata_type: Option<String>,
    pub declared_sha256: Option<String>,
}

/// Grouped arguments for the inverse restore CAS.
#[derive(Debug, Clone)]
pub struct RestoreTaskItemArtifactInlineRequest<'a> {
    pub item_id: Uuid,
    pub kind: TaskItemArtifactKind,
    pub storage_object_id: Uuid,
    pub sha256: &'a str,
    pub size_bytes: i64,
    pub storage_backend: &'a str,
    pub encoding: &'a str,
    /// Base64 text restored for a `download` kind.
    pub download_base64: Option<&'a str>,
    /// Ingest-section array value restored for a `sections` kind.
    pub sections_json: Option<serde_json::Value>,
}

/// Point-in-time count of the references that remain after a restore batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemainingArtifactReferences {
    /// Every artifact row, including references under a soft-deleted parent.
    pub total: i64,
    pub download: i64,
    pub sections: i64,
    pub active: i64,
    pub terminal: i64,
    pub soft_deleted: i64,
}

/// Keyset cursor over the deterministic `(item_id, kind)` scan order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactBackfillCursor {
    pub item_id: Uuid,
    pub kind: TaskItemArtifactKind,
}

/// Grouped arguments for the fingerprinted terminal attach.
#[derive(Debug, Clone, Copy)]
pub struct AttachTerminalTaskItemArtifactRequest<'a> {
    pub item_id: Uuid,
    pub kind: TaskItemArtifactKind,
    pub storage_object_id: Uuid,
    pub sha256: &'a str,
    pub size_bytes: i64,
    pub storage_backend: &'a str,
    pub field_fingerprint: &'a str,
}

/// Raw row shape for the two metadata scans. The public row resolves `kind`
/// into its typed form.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ArtifactBackfillRecord {
    item_id: Option<Uuid>,
    kind: Option<String>,
    task_id: Option<Uuid>,
    group_id: Option<i64>,
    item_status: Option<String>,
    task_status: Option<String>,
    task_deleted: Option<bool>,
    lease_active: Option<bool>,
    field_type: Option<String>,
    encoded_len: Option<i64>,
    declared_sha256_present: Option<bool>,
    declared_sha256_valid: Option<bool>,
    artifact_present: Option<bool>,
}

impl From<ArtifactBackfillRecord> for ArtifactBackfillRow {
    fn from(record: ArtifactBackfillRecord) -> Self {
        let kind = record.kind.expect("metadata scan returns a kind");
        Self {
            item_id: record.item_id.expect("metadata scan returns an item id"),
            kind: TaskItemArtifactKind::from_parts(&kind, encoding_for(&kind))
                .expect("artifact backfill queries return a supported kind"),
            task_id: record.task_id.expect("metadata scan returns a task id"),
            group_id: record.group_id,
            item_status: record
                .item_status
                .expect("metadata scan returns an item status"),
            task_status: record
                .task_status
                .expect("metadata scan returns a task status"),
            task_deleted: record.task_deleted.unwrap_or(false),
            lease_active: record.lease_active.unwrap_or(false),
            field_type: record.field_type,
            encoded_len: record.encoded_len,
            declared_sha256_present: record.declared_sha256_present.unwrap_or(false),
            declared_sha256_valid: record.declared_sha256_valid.unwrap_or(false),
            artifact_present: record.artifact_present.unwrap_or(false),
        }
    }
}

/// Raw row shape for the single-field apply fetch.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ArtifactBackfillFieldRecord {
    item_id: Option<Uuid>,
    group_id: Option<i64>,
    item_status: Option<String>,
    task_status: Option<String>,
    task_deleted: Option<bool>,
    lease_active: Option<bool>,
    field_type: Option<String>,
    field_text: Option<String>,
    declared_sha256: Option<String>,
    skip_reason: Option<String>,
}

/// Raw row shape for the inverse scan.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ArtifactRestoreRecord {
    item_id: Option<Uuid>,
    task_id: Option<Uuid>,
    kind: Option<String>,
    encoding: Option<String>,
    storage_object_id: Option<Uuid>,
    object_group_id: Option<i64>,
    sha256: Option<String>,
    size_bytes: Option<i64>,
    storage_backend: Option<String>,
    item_status: Option<String>,
    task_status: Option<String>,
    task_deleted: Option<bool>,
    lease_active: Option<bool>,
    task_group_id: Option<i64>,
    inline_type: Option<String>,
    download_metadata_type: Option<String>,
    declared_sha256: Option<String>,
}

impl From<ArtifactRestoreRecord> for ArtifactRestoreRow {
    fn from(record: ArtifactRestoreRecord) -> Self {
        let kind = record.kind.expect("restore scan returns a kind");
        Self {
            item_id: record.item_id.expect("restore scan returns an item id"),
            task_id: record.task_id.expect("restore scan returns a task id"),
            kind: TaskItemArtifactKind::from_parts(
                &kind,
                &record.encoding.clone().unwrap_or_default(),
            )
            .expect("restore scan returns a supported kind/encoding pair"),
            encoding: record.encoding.expect("restore scan returns an encoding"),
            storage_object_id: record
                .storage_object_id
                .expect("restore scan returns an object id"),
            object_group_id: record
                .object_group_id
                .expect("restore scan returns a registry group"),
            object_sha256: record
                .sha256
                .expect("restore scan returns a registry digest"),
            object_size_bytes: record
                .size_bytes
                .expect("restore scan returns a registry size"),
            object_storage_backend: record
                .storage_backend
                .expect("restore scan returns a registry backend"),
            item_status: record
                .item_status
                .expect("restore scan returns an item status"),
            task_status: record
                .task_status
                .expect("restore scan returns a task status"),
            task_deleted: record.task_deleted.unwrap_or(false),
            lease_active: record.lease_active.unwrap_or(false),
            task_group_id: record.task_group_id,
            inline_type: record.inline_type,
            download_metadata_type: record.download_metadata_type,
            declared_sha256: record.declared_sha256,
        }
    }
}

fn encoding_for(kind: &str) -> &'static str {
    match kind {
        "download" => "raw-v1",
        "sections" => "ingest-sections-json-v1",
        _ => "unknown",
    }
}

fn cursor_parts(cursor: Option<ArtifactBackfillCursor>) -> (Option<Uuid>, Option<&'static str>) {
    match cursor {
        Some(cursor) => (Some(cursor.item_id), Some(cursor.kind.kind())),
        None => (None, None),
    }
}

impl Database {
    /// Whether the #975 artifact relation exists in this database.
    pub async fn task_item_artifact_schema_present(&self) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/artifact_backfill/detect_artifact_schema.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }

    /// One metadata page that never references the artifact relation.
    pub async fn list_inline_artifact_candidates(
        &self,
        kind_filter: &str,
        cursor: Option<ArtifactBackfillCursor>,
        limit: i64,
        item_ids: Option<&[Uuid]>,
    ) -> Result<Vec<ArtifactBackfillRow>> {
        let (cursor_id, cursor_kind) = cursor_parts(cursor);
        let rows = sqlx::query_file_as!(
            ArtifactBackfillRecord,
            "src/sql/db/tasks/artifact_backfill/list_inline_candidates.sql",
            kind_filter,
            cursor_id,
            cursor_kind,
            limit,
            item_ids
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// One metadata page that also distinguishes an existing artifact
    /// reference. Only call after `task_item_artifact_schema_present`.
    pub async fn list_terminal_artifact_states(
        &self,
        kind_filter: &str,
        cursor: Option<ArtifactBackfillCursor>,
        limit: i64,
        item_ids: Option<&[Uuid]>,
    ) -> Result<Vec<ArtifactBackfillRow>> {
        let (cursor_id, cursor_kind) = cursor_parts(cursor);
        let rows = sqlx::query_file_as!(
            ArtifactBackfillRecord,
            "src/sql/db/tasks/artifact_backfill/list_terminal_artifact_states.sql",
            kind_filter,
            cursor_id,
            cursor_kind,
            limit,
            item_ids
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Fetch the single inline field of one item/kind once.
    pub async fn fetch_artifact_backfill_field(
        &self,
        item_id: Uuid,
        kind: TaskItemArtifactKind,
        max_artifact_bytes: i64,
    ) -> Result<Option<ArtifactBackfillField>> {
        let Some(row) = sqlx::query_file_as!(
            ArtifactBackfillFieldRecord,
            "src/sql/db/tasks/artifact_backfill/fetch_artifact_field.sql",
            item_id,
            kind.kind(),
            max_artifact_bytes
        )
        .fetch_optional(self.pool())
        .await?
        else {
            return Ok(None);
        };
        Ok(Some(ArtifactBackfillField {
            item_id: row.item_id.expect("field fetch returns an item id"),
            group_id: row.group_id,
            item_status: row.item_status.expect("field fetch returns an item status"),
            task_status: row.task_status.expect("field fetch returns a task status"),
            task_deleted: row.task_deleted.unwrap_or(false),
            lease_active: row.lease_active.unwrap_or(false),
            field_type: row.field_type,
            field_text: row.field_text,
            declared_sha256: row.declared_sha256,
            skip_reason: row.skip_reason,
        }))
    }

    /// One metadata page of existing artifact references for the inverse
    /// direction. Only call after `task_item_artifact_schema_present`.
    pub async fn list_artifact_restore_candidates(
        &self,
        kind_filter: &str,
        cursor: Option<ArtifactBackfillCursor>,
        limit: i64,
        item_ids: Option<&[Uuid]>,
    ) -> Result<Vec<ArtifactRestoreRow>> {
        let (cursor_id, cursor_kind) = cursor_parts(cursor);
        let rows = sqlx::query_file_as!(
            ArtifactRestoreRecord,
            "src/sql/db/tasks/artifact_backfill/restore_candidates.sql",
            kind_filter,
            cursor_id,
            cursor_kind,
            limit,
            item_ids
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Restore one missing inline field and delete exactly its reference.
    ///
    /// Returns false without mutating anything when the item, parent, lease,
    /// group, registry identity or inline state no longer matches.
    pub async fn restore_task_item_artifact_inline(
        &self,
        request: RestoreTaskItemArtifactInlineRequest<'_>,
    ) -> Result<bool> {
        let restored: i64 = sqlx::query_file_scalar!(
            "src/sql/db/tasks/artifact_backfill/restore_inline.sql",
            request.item_id,
            request.kind.kind(),
            request.storage_object_id,
            request.sha256,
            request.size_bytes,
            request.storage_backend,
            request.encoding,
            request.download_base64,
            request.sections_json
        )
        .fetch_one(self.pool())
        .await?;
        Ok(restored > 0)
    }

    /// Point-in-time count of the artifact references that remain.
    pub async fn remaining_artifact_references(&self) -> Result<RemainingArtifactReferences> {
        let row = sqlx::query_file!(
            "src/sql/db/tasks/artifact_backfill/remaining_artifact_references.sql"
        )
        .fetch_one(self.pool())
        .await?;
        Ok(RemainingArtifactReferences {
            total: row.total,
            download: row.download,
            sections: row.sections,
            active: row.active,
            terminal: row.terminal,
            soft_deleted: row.soft_deleted,
        })
    }

    /// Commit one terminal artifact reference and remove only its inline field.
    ///
    /// Returns false without mutating anything when the item, parent, lease,
    /// group, registry identity or inline fingerprint no longer matches.
    pub async fn attach_terminal_task_item_artifact(
        &self,
        request: AttachTerminalTaskItemArtifactRequest<'_>,
    ) -> Result<bool> {
        let attached: i64 = sqlx::query_file_scalar!(
            "src/sql/db/tasks/artifact_backfill/attach_terminal_task_item_artifact.sql",
            request.item_id,
            request.kind.kind(),
            request.storage_object_id,
            request.sha256,
            request.size_bytes,
            request.storage_backend,
            request.field_fingerprint,
            request.kind.encoding()
        )
        .fetch_one(self.pool())
        .await?;
        Ok(attached > 0)
    }

    /// Applied ledger checksum for one migration version, when the ledger row
    /// exists. The boolean is the ledger's success flag.
    pub async fn applied_migration_checksum(
        &self,
        version: i64,
    ) -> Result<Option<(Vec<u8>, bool)>> {
        let Some(row) = sqlx::query_file!(
            "src/sql/db/tasks/artifact_backfill/fetch_applied_migration.sql",
            version
        )
        .fetch_optional(self.pool())
        .await?
        else {
            return Ok(None);
        };
        Ok(Some((row.checksum, row.success)))
    }
}
