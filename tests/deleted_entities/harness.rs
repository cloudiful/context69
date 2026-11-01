//! Shared fixture accessors for the entity tombstone tests (issue #1138 P24).
//!
//! The tests share one scratch database, so every fixture is namespaced by a
//! unique id and removed again when the test finishes. A tombstone is global
//! state by design — it excludes its entity from every search — so the
//! cleanup is part of the fixture, not an afterthought.

use anyhow::Result;
use context69::db::Database;
use sqlx::PgConnection;
use uuid::Uuid;

/// A group that owns the library files of one test.
///
/// The `library_files.group_id` reference is a real foreign key, so the file
/// fixtures need a real group row. The group is removed with the files it
/// owns.
pub struct GroupFixture {
    pub id: i64,
}

impl GroupFixture {
    pub async fn create(db: &Database) -> Result<Self> {
        let group_key = format!("p24-tombstone-{}", Uuid::new_v4());
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
             VALUES ($1, $1, 'public', 'shared', $1) RETURNING id",
        )
        .bind(&group_key)
        .fetch_one(db.pool())
        .await?;
        Ok(Self { id })
    }

    pub async fn remove(&self, db: &Database) -> Result<()> {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(self.id)
            .execute(db.pool())
            .await?;
        Ok(())
    }
}

/// A library file row that owns no document and no storage object.
pub struct FileFixture {
    pub id: Uuid,
}

impl FileFixture {
    pub async fn create(db: &Database, group_id: i64) -> Result<Self> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.library_files (id, filename, media_type, size_bytes, sha256, \
             storage_rel_path, ingest_status, group_id, visibility, external_id) \
             VALUES ($1, $2, 'text/plain', 1, $3, $4, 'pending', $5, 'public', $2)",
        )
        .bind(id)
        .bind(format!("p24-{id}.txt"))
        .bind(format!("sha-{id}"))
        .bind(format!("p24/{id}.txt"))
        .bind(group_id)
        .execute(db.pool())
        .await?;
        Ok(Self { id })
    }

    pub async fn remove(&self, db: &Database) -> Result<()> {
        sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
            .bind(self.id)
            .execute(db.pool())
            .await?;
        Ok(())
    }
}

/// Remove the tombstones of one test.
///
/// The rows are removed by their exact ids, never by a pattern, so a
/// concurrent test keeps its own exclusions.
pub async fn clear_tombstones(db: &Database, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM context69.library_deleted_entities WHERE id = ANY($1)")
        .bind(ids)
        .execute(db.pool())
        .await?;
    Ok(())
}

/// The ids of the tombstones of the given entities.
pub async fn tombstone_ids(
    db: &Database,
    file_ids: &[Uuid],
    document_ids: &[i64],
) -> Result<Vec<i64>> {
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM context69.library_deleted_entities \
         WHERE file_id = ANY($1) OR document_id = ANY($2) ORDER BY id",
    )
    .bind(file_ids)
    .bind(document_ids)
    .fetch_all(db.pool())
    .await?;
    Ok(rows)
}

/// The next value of the document id sequence, consumed without a row.
pub async fn peek_next_document_id(db: &Database) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT nextval('context69.documents_id_seq'::regclass)")
            .fetch_one(db.pool())
            .await?,
    )
}

/// Tombstone a run of document ids in one transaction.
pub async fn tombstone_document_ids(db: &Database, document_ids: &[i64]) -> Result<Vec<i64>> {
    let mut tx = db.pool().begin().await?;
    let ids = sqlx::query_scalar::<_, i64>(
        "INSERT INTO context69.library_deleted_entities (document_id) \
         SELECT unnest($1::bigint[]) RETURNING id",
    )
    .bind(document_ids)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ids)
}

/// The registry epoch of the database.
pub async fn registry_epoch(db: &Database) -> Result<i64> {
    db.read_search_registry_epoch().await
}

/// The search generation of the database.
pub async fn search_generation(db: &Database) -> Result<i64> {
    db.get_search_generation().await
}

/// Commit a tombstone of one deleted file on a borrowed connection.
pub async fn tombstone_file(
    db: &Database,
    connection: &mut PgConnection,
    file_id: Uuid,
    group_id: i64,
) -> Result<bool> {
    db.insert_deleted_file_entity_in(connection, file_id, Some(group_id))
        .await
}
