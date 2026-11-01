use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::library_store::{TaskWriteFence, TaskWriteTx};

use super::{Database, lock_file_processing_slots};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestRunMode {
    FirstInsert,
    Replacement,
}

impl IngestRunMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstInsert => "first_insert",
            Self::Replacement => "replacement",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestRunSource {
    Url,
    Stored,
}

impl IngestRunSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Stored => "stored",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NewIngestRun {
    pub mode: IngestRunMode,
    pub source: IngestRunSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryIngestRun {
    pub item_id: Uuid,
    pub task_id: Uuid,
    pub file_id: Uuid,
    pub batch_id: Uuid,
    pub generation: i64,
    pub mode: IngestRunMode,
    pub source: IngestRunSource,
    pub lease_token: Uuid,
    pub execution_task_id: Uuid,
    pub execution_generation: i64,
    pub resolved_at: Option<DateTime<Utc>>,
    pub retired_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RunSectionRow {
    file_id: Uuid,
    section_external_id: String,
    document_id: i64,
}

struct RunRow {
    item_id: Uuid,
    task_id: Uuid,
    file_id: Uuid,
    batch_id: Uuid,
    generation: i64,
    mode: String,
    source: String,
    lease_token: Uuid,
    execution_task_id: Uuid,
    execution_generation: i64,
    resolved_at: Option<DateTime<Utc>>,
    retired_at: Option<DateTime<Utc>>,
}

impl TryFrom<RunRow> for LibraryIngestRun {
    type Error = anyhow::Error;

    fn try_from(row: RunRow) -> Result<Self> {
        let mode = match row.mode.as_str() {
            "first_insert" => IngestRunMode::FirstInsert,
            "replacement" => IngestRunMode::Replacement,
            _ => bail!("unknown ingest run mode"),
        };
        let source = match row.source.as_str() {
            "url" => IngestRunSource::Url,
            "stored" => IngestRunSource::Stored,
            _ => bail!("unknown ingest run source"),
        };
        Ok(Self {
            item_id: row.item_id,
            task_id: row.task_id,
            file_id: row.file_id,
            batch_id: row.batch_id,
            generation: row.generation,
            mode,
            source,
            lease_token: row.lease_token,
            execution_task_id: row.execution_task_id,
            execution_generation: row.execution_generation,
            resolved_at: row.resolved_at,
            retired_at: row.retired_at,
        })
    }
}

async fn verify_item(write: &mut TaskWriteTx<'_>, task_id: Uuid, file_id: Uuid) -> Result<()> {
    let fence = write.fence();
    let item = sqlx::query_file_scalar!(
        "src/sql/db/library_ingest_runs/verify_item.sql",
        fence.item_id(),
        task_id,
        file_id,
        fence.token(),
        fence.execution_task_id(),
        fence.generation()
    )
    .fetch_optional(write.connection())
    .await?;
    ensure!(
        item.is_some(),
        "task item association or lease is no longer valid"
    );
    Ok(())
}

impl Database {
    /// Locks the identity of one library file creation.
    ///
    /// The identity of a file in a group is its external id and its content
    /// digest, so both keys are locked in one canonical order. Two writers that
    /// would create the same identity therefore serialize instead of racing,
    /// and the second one re-probes the identity after the first one committed.
    pub async fn lock_library_identity_in(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        external_id: Option<&str>,
        sha256: &str,
    ) -> Result<()> {
        let external_key = format!("library:{group_id}:external:{}", external_id.unwrap_or(""));
        let digest_key = format!("library:{group_id}:sha:{sha256}");
        sqlx::query_file_scalar!(
            "src/sql/db/library_ingest_runs/lock_identity.sql",
            external_key,
            digest_key
        )
        .fetch_one(connection)
        .await
        .context("lock the library file identity")?;
        Ok(())
    }

    /// Allocates one generation from the dedicated library generation sequence.
    ///
    /// A translation publication has no durable ingest run, so it takes an
    /// independent generation per attempt and scopes its chunk ids with it. A
    /// rolled-back transaction only leaves a harmless sequence gap.
    pub async fn next_ingest_generation_in(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/library_ingest_runs/next_generation.sql")
                .fetch_one(connection)
                .await?,
        )
    }

    pub async fn claim_or_create_library_ingest_run_in(
        &self,
        write: &mut TaskWriteTx<'_>,
        task_id: Uuid,
        file_id: Uuid,
        requested: Option<NewIngestRun>,
    ) -> Result<LibraryIngestRun> {
        verify_item(write, task_id, file_id).await?;
        let fence = write.fence();
        let row = sqlx::query_file_as!(
            RunRow,
            "src/sql/db/library_ingest_runs/read_for_update.sql",
            fence.item_id()
        )
        .fetch_optional(write.connection())
        .await?;
        if let Some(row) = row {
            let existing = LibraryIngestRun::try_from(row)?;
            ensure!(
                existing.task_id == task_id && existing.file_id == file_id,
                "ingest run identity mismatch"
            );
            if let Some(requested) = requested {
                ensure!(
                    existing.mode == requested.mode && existing.source == requested.source,
                    "ingest run disposition mismatch"
                );
            }
            if existing.resolved_at.is_some() {
                return Ok(existing);
            }
            let row = sqlx::query_file_as!(
                RunRow,
                "src/sql/db/library_ingest_runs/adopt.sql",
                fence.item_id(),
                fence.token(),
                fence.execution_task_id(),
                fence.generation()
            )
            .fetch_one(write.connection())
            .await?;
            return row.try_into();
        }
        let requested =
            requested.ok_or_else(|| anyhow::anyhow!("new ingest run requires disposition"))?;
        let batch_id = Uuid::new_v4();
        let row = sqlx::query_file_as!(
            RunRow,
            "src/sql/db/library_ingest_runs/insert.sql",
            fence.item_id(),
            task_id,
            file_id,
            batch_id,
            requested.mode.as_str(),
            requested.source.as_str(),
            fence.token(),
            fence.execution_task_id(),
            fence.generation()
        )
        .fetch_one(write.connection())
        .await?;
        row.try_into()
    }

    /// The current durable run of one item, checked under its item lock.
    ///
    /// The caller already holds the fenced write of the item, so the row is
    /// read with the item locked: a claim, adopt, retirement or switch cannot
    /// commit between this check and the mutation that follows it. The run must
    /// still be the exact batch, generation, file, mode and source the caller
    /// prepared against, unresolved and not retired. A retired run is already
    /// history and a resolved run belongs to a finished switch, so neither may
    /// carry a new mapping or admission.
    pub async fn verify_current_library_ingest_run_in(
        &self,
        write: &mut TaskWriteTx<'_>,
        expected: &LibraryIngestRun,
    ) -> Result<LibraryIngestRun> {
        let current = self
            .read_library_ingest_run_in(write)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the durable ingest run is no longer current"))?;
        ensure!(
            current.batch_id == expected.batch_id
                && current.generation == expected.generation
                && current.file_id == expected.file_id
                && current.mode == expected.mode
                && current.source == expected.source,
            "the durable ingest run identity changed"
        );
        ensure!(
            current.retired_at.is_none(),
            "the durable ingest run was retired"
        );
        ensure!(
            current.resolved_at.is_none(),
            "the durable ingest run is already resolved"
        );
        Ok(current)
    }

    /// The final document id of one new section of a durable ingest run.
    ///
    /// The first call reserves an id from the document sequence and records it
    /// under the run and the section key in the same fenced transaction, so the
    /// decision is durable before the caller writes any external point. The
    /// item is locked and the run rechecked before the reservation, so a
    /// terminal retirement that committed first refuses this mutation instead
    /// of leaving a mapping on a retired run.
    ///
    /// Every later call of the same run reuses that id; a changed section
    /// identity or another file refuses instead of remapping, and a terminal
    /// retry that claimed a fresh generation owns a different run and therefore
    /// allocates its own ids.
    pub async fn reserve_library_ingest_run_section(
        &self,
        fence: TaskWriteFence,
        run: &LibraryIngestRun,
        section_key: &str,
        section_external_id: &str,
    ) -> Result<i64> {
        let mut write = crate::library_store::begin_task_write(self, fence).await?;
        self.verify_current_library_ingest_run_in(&mut write, run)
            .await?;
        let existing = sqlx::query_file_as!(
            RunSectionRow,
            "src/sql/db/library_ingest_runs/read_section_identity.sql",
            run.batch_id,
            section_key
        )
        .fetch_optional(write.connection())
        .await?;
        let document_id = if let Some(existing) = existing {
            ensure!(
                existing.file_id == run.file_id,
                "the durable run already reserved a document for the section {section_key} of \
                 another file"
            );
            ensure!(
                existing.section_external_id == section_external_id,
                "the durable run already reserved the document {} for another section identity",
                existing.document_id
            );
            existing.document_id
        } else {
            let document_id = self.reserve_document_id_on(write.connection()).await?;
            sqlx::query_file_scalar!(
                "src/sql/db/library_ingest_runs/insert_section_identity.sql",
                run.batch_id,
                section_key,
                run.file_id,
                section_external_id,
                document_id
            )
            .fetch_one(write.connection())
            .await?;
            document_id
        };
        ensure!(
            write.commit().await?,
            "the grant no longer authorizes the section identity reservation"
        );
        Ok(document_id)
    }

    /// Whether a library file carries an active unresolved ingest run.
    ///
    /// An active run is a current row that was never resolved: a retired row
    /// is history and a resolved row belongs to a finished switch, so
    /// neither blocks a delete. `exclude_item_id` ignores the run of one
    /// task item, so a worker may delete the file of its own run.
    pub async fn file_has_active_unresolved_run_in(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        exclude_item_id: Option<Uuid>,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/library_ingest_runs/read_active_unresolved_file.sql",
            file_id,
            exclude_item_id
        )
        .fetch_one(&mut *connection)
        .await
        .context("read the active ingest run of a file")?)
    }

    /// Claims the durable replacement run of an ordinary single-file update.
    ///
    /// The per-file processing slot is locked first, so the foreign active-run
    /// check cannot race another item starting a replacement of the same file.
    /// A foreign active run returns `Ok(None)` without binding the item file;
    /// the caller rolls the guarded write back and refuses retryably.
    pub async fn begin_ordinary_replacement_run_in(
        &self,
        write: &mut TaskWriteTx<'_>,
        task_id: Uuid,
        file_id: Uuid,
        source: IngestRunSource,
    ) -> Result<Option<LibraryIngestRun>> {
        lock_file_processing_slots(write.connection(), &[file_id]).await?;
        let foreign = sqlx::query_file_scalar!(
            "src/sql/db/library_ingest_runs/read_foreign_active.sql",
            file_id,
            write.fence().item_id()
        )
        .fetch_optional(write.connection())
        .await?;
        if foreign.is_some() {
            return Ok(None);
        }
        let fence = write.fence();
        let bound = sqlx::query_file_scalar!(
            "src/sql/db/tasks/set_task_item_file_keep_input.sql",
            fence.item_id(),
            fence.token(),
            file_id,
            fence.execution_task_id(),
            fence.generation()
        )
        .fetch_optional(write.connection())
        .await?
        .is_some();
        ensure!(bound, "task item association or lease is no longer valid");
        self.claim_or_create_library_ingest_run_in(
            write,
            task_id,
            file_id,
            Some(NewIngestRun {
                mode: IngestRunMode::Replacement,
                source,
            }),
        )
        .await
        .map(Some)
    }

    /// Whether a file already carries another task item's durable ingest
    /// history. Translation-only registry rows have no run and never count.
    pub async fn file_has_foreign_ingest_history(
        &self,
        file_id: Uuid,
        item_id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/library_ingest_runs/file_foreign_history.sql",
            file_id,
            item_id
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn mark_library_ingest_run_resolved_in(
        &self,
        write: &mut TaskWriteTx<'_>,
        task_id: Uuid,
        file_id: Uuid,
    ) -> Result<bool> {
        verify_item(write, task_id, file_id).await?;
        let fence = write.fence();
        let row = sqlx::query_file_scalar!(
            "src/sql/db/library_ingest_runs/resolve.sql",
            fence.item_id(),
            fence.token(),
            fence.execution_task_id(),
            fence.generation()
        )
        .fetch_optional(write.connection())
        .await?;
        Ok(row.is_some())
    }

    pub async fn read_library_ingest_run_in(
        &self,
        write: &mut TaskWriteTx<'_>,
    ) -> Result<Option<LibraryIngestRun>> {
        let item_id = write.fence().item_id();
        sqlx::query_file_as!(
            RunRow,
            "src/sql/db/library_ingest_runs/read_for_update.sql",
            item_id
        )
        .fetch_optional(write.connection())
        .await?
        .map(TryInto::try_into)
        .transpose()
    }

    pub async fn read_library_ingest_run(&self, item_id: Uuid) -> Result<Option<LibraryIngestRun>> {
        sqlx::query_file_as!(RunRow, "src/sql/db/library_ingest_runs/read.sql", item_id)
            .fetch_optional(self.pool())
            .await?
            .map(TryInto::try_into)
            .transpose()
    }

    /// Every run row of the item, oldest generation first.
    ///
    /// Retired rows stay readable as history; only the row with no
    /// `retired_at` is current, and only that row can be adopted, resolved or
    /// returned by [`Self::read_library_ingest_run`].
    pub async fn read_library_ingest_run_history(
        &self,
        item_id: Uuid,
    ) -> Result<Vec<LibraryIngestRun>> {
        sqlx::query_file_as!(
            RunRow,
            "src/sql/db/library_ingest_runs/read_history.sql",
            item_id
        )
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}
