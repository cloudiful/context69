use anyhow::{Context, Result};

use super::Database;

/// The one physical collection ordinary point writers may use.
///
/// The fingerprint is the desired logical identity the deployment was
/// configured with, while `collection_name` is the physical collection that
/// identity was built into. The two may differ after an explicit migration, so
/// callers compare the fingerprint and the physical name together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveVectorIndexState {
    pub collection_name: String,
    pub fingerprint: String,
    pub dimensions: usize,
}

impl Database {
    pub async fn get_vector_index_fingerprint(
        &self,
        collection_name: &str,
    ) -> Result<Option<String>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/vector_index_state/get.sql", collection_name)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// The active vector index, read on the caller's connection.
    ///
    /// An ordinary write admission reads it inside its own transaction, under
    /// the registry epoch lock, so the pointer it validates cannot change
    /// between the check and the write.
    pub async fn read_active_vector_index_state_in(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<Option<ActiveVectorIndexState>> {
        let row = sqlx::query_file!("src/sql/db/vector_index_state/get_active.sql")
            .fetch_optional(&mut *connection)
            .await
            .context("read the active vector index")?;
        row.map(|row| {
            let dimensions = usize::try_from(row.dimensions)
                .context("the active vector dimensions must be non-negative")?;
            Ok(ActiveVectorIndexState {
                collection_name: row.collection_name,
                fingerprint: row.fingerprint,
                dimensions,
            })
        })
        .transpose()
    }

    /// The active vector index, read on the pool.
    pub async fn read_active_vector_index_state(&self) -> Result<Option<ActiveVectorIndexState>> {
        let mut connection = self.pool.acquire().await?;
        self.read_active_vector_index_state_in(&mut connection)
            .await
    }

    /// Return the active pointer to no live collection on the caller's
    /// connection.
    pub async fn deactivate_active_vector_index_state_in(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<()> {
        sqlx::query_file!("src/sql/db/vector_index_state/deactivate_active.sql")
            .execute(&mut *connection)
            .await
            .context("deactivate the active vector index")?;
        Ok(())
    }

    /// Activate one recorded physical collection on the caller's connection.
    pub async fn activate_vector_index_state_in(
        &self,
        connection: &mut sqlx::PgConnection,
        collection_name: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/vector_index_state/activate.sql",
            collection_name
        )
        .execute(&mut *connection)
        .await
        .context("activate the vector index")?;
        Ok(())
    }

    pub async fn save_vector_index_state(&self, state: &VectorIndexState<'_>) -> Result<()> {
        let mut connection = self.pool.acquire().await?;
        self.save_vector_index_state_in(&mut connection, state)
            .await
    }

    /// Write the identity a rebuild produced on the caller's connection.
    ///
    /// The rebuild completion writes the identity, clears the rebuild barrier
    /// and moves the epochs in one transaction, so the identity of a finished
    /// rebuild and the barrier that describes it can never disagree. The save
    /// never changes the active pointer: a newly recorded target stays inactive
    /// until the activation step publishes it.
    pub async fn save_vector_index_state_in(
        &self,
        connection: &mut sqlx::PgConnection,
        state: &VectorIndexState<'_>,
    ) -> Result<()> {
        let dimensions =
            i64::try_from(state.dimensions).context("embedding dimensions too large")?;
        let rebuilt_chunks =
            i64::try_from(state.rebuilt_chunks).context("rebuilt chunk count too large")?;
        sqlx::query_file!(
            "src/sql/db/vector_index_state/save.sql",
            state.collection_name,
            state.fingerprint,
            state.embedding_base_url,
            state.embedding_model,
            dimensions,
            rebuilt_chunks
        )
        .execute(&mut *connection)
        .await?;
        Ok(())
    }
}

pub struct VectorIndexState<'a> {
    pub collection_name: &'a str,
    pub fingerprint: &'a str,
    pub embedding_base_url: &'a str,
    pub embedding_model: &'a str,
    pub dimensions: usize,
    pub rebuilt_chunks: usize,
}
