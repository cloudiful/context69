//! The SQL boundary of the secret store.
//!
//! This module owns secret persistence and its row projections. Native queries
//! share the existing pool; the startup backfill uses borrowed SQLx transactions.
//! Interpretation and error mapping belong to the store above.

pub(crate) mod backfill;

use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use sqlx::PgPool;

use crate::entity::internal_secrets::{Column, Entity};
use crate::rows::{StoredSecret, StoredSecretBytes, StoredSecretMetadata};

fn database_error(error: sea_orm::DbErr) -> sqlx::Error {
    use sea_orm::{DbErr, RuntimeErr};
    let error = match error {
        DbErr::Exec(RuntimeErr::SqlxError(error))
        | DbErr::Query(RuntimeErr::SqlxError(error))
        | DbErr::Conn(RuntimeErr::SqlxError(error)) => error,
        error => return sqlx::Error::Protocol(error.to_string()),
    };
    match std::sync::Arc::try_unwrap(error) {
        Ok(error) => error,
        Err(error) => sqlx::Error::Protocol(error.to_string()),
    }
}

/// The pool the secret store reads and writes through.
///
/// This is a separate handle from the application's own database type on
/// purpose: the store must stay usable by a crate that has no dependency on the
/// application, and it must not be able to reach any table it does not own.
#[derive(Clone)]
pub struct SecretDatabase {
    pool: PgPool,
    orm: sea_orm::DatabaseConnection,
}

impl std::fmt::Debug for SecretDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretDatabase")
            .field("pool", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl SecretDatabase {
    /// Wraps an existing pool. No migration, connection or write happens here.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            orm: pool.clone().into(),
            pool,
        }
    }

    /// The pool, for a caller that needs to compose a transaction around the
    /// store.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Reads one secret row, or `None` when the key is not stored.
    ///
    /// This is the only statement that selects `value`, so it is the only path
    /// that can move a secret's bytes.
    pub async fn get_secret(&self, key_name: &str) -> Result<Option<StoredSecret>, sqlx::Error> {
        let row = Entity::find_by_id(key_name.to_owned())
            .select_only()
            .columns([
                Column::Value,
                Column::Purpose,
                Column::KeyVersion,
                Column::CiphertextVersion,
            ])
            .into_tuple::<(Vec<u8>, Option<String>, i32, i32)>()
            .one(&self.orm)
            .await
            .map_err(database_error)?;
        Ok(row.map(
            |(value, purpose, key_version, ciphertext_version)| StoredSecret {
                value,
                purpose,
                key_version,
                ciphertext_version,
            },
        ))
    }

    /// Reads one secret row's metadata, without its value.
    ///
    /// Backs the presence check: a deployment that has no master key still
    /// needs `has_*` to be truthful, and that must not require decrypting
    /// anything.
    pub async fn get_secret_metadata(
        &self,
        key_name: &str,
    ) -> Result<Option<StoredSecretMetadata>, sqlx::Error> {
        let row = Entity::find_by_id(key_name.to_owned())
            .select_only()
            .columns([Column::Purpose, Column::CiphertextVersion])
            .into_tuple::<(Option<String>, i32)>()
            .one(&self.orm)
            .await
            .map_err(database_error)?;
        Ok(
            row.map(|(purpose, ciphertext_version)| StoredSecretMetadata {
                purpose,
                ciphertext_version,
            }),
        )
    }

    /// Creates a secret row and reports whether this call created it.
    ///
    /// `false` means the key was already taken and nothing was written, so the
    /// caller can read the existing value instead of overwriting it.
    pub async fn put_secret(
        &self,
        key_name: &str,
        stored: &StoredWrite<'_>,
    ) -> Result<bool, sqlx::Error> {
        use sea_orm::ConnectionTrait;
        use sea_orm::sea_query::{Alias, Query};
        let statement = Query::insert()
            .into_table((Alias::new("context69"), Entity))
            .columns([
                Column::Key,
                Column::Value,
                Column::Purpose,
                Column::KeyVersion,
                Column::CiphertextVersion,
                Column::UpdatedAt,
            ])
            .values_panic([
                key_name.into(),
                stored.value.to_vec().into(),
                stored.purpose.map(str::to_owned).into(),
                stored.key_version.into(),
                stored.ciphertext_version.into(),
                Expr::current_timestamp(),
            ])
            .on_conflict(OnConflict::column(Column::Key).do_nothing().to_owned())
            .to_owned();
        let result = self.orm.execute(&statement).await.map_err(database_error)?;
        Ok(result.rows_affected() > 0)
    }

    /// Replaces the stored representation of an existing secret.
    ///
    /// `false` means no row carried that key, so nothing was written.
    pub async fn rotate_secret(
        &self,
        key_name: &str,
        stored: &StoredWrite<'_>,
    ) -> Result<bool, sqlx::Error> {
        let result = Entity::update_many()
            .col_expr(Column::Value, Expr::value(stored.value.to_vec()))
            .col_expr(
                Column::Purpose,
                Expr::value(stored.purpose.map(str::to_owned)),
            )
            .col_expr(Column::KeyVersion, Expr::value(stored.key_version))
            .col_expr(
                Column::CiphertextVersion,
                Expr::value(stored.ciphertext_version),
            )
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Key.eq(key_name))
            .exec(&self.orm)
            .await
            .map_err(database_error)?;
        Ok(result.rows_affected > 0)
    }

    /// Deletes a secret row and reports whether this call deleted it.
    pub async fn delete_secret(&self, key_name: &str) -> Result<bool, sqlx::Error> {
        let result = Entity::delete_by_id(key_name.to_owned())
            .exec(&self.orm)
            .await
            .map_err(database_error)?;
        Ok(result.rows_affected > 0)
    }

    /// Lists the sealed keys one purpose owns at one master-key version.
    ///
    /// The rewrap worklist. Two properties matter and both live in the
    /// statement rather than in the caller: it projects the key name only, so
    /// building the list moves no secret, and it filters on the key version, so
    /// a row an earlier run already re-sealed is not offered a second time.
    pub async fn list_sealed_secret_keys(
        &self,
        purpose: &str,
        key_version: i32,
    ) -> Result<Vec<String>, sqlx::Error> {
        Entity::find()
            .select_only()
            .column(Column::Key)
            .filter(Column::Purpose.eq(purpose))
            .filter(Column::KeyVersion.eq(key_version))
            .filter(Column::CiphertextVersion.eq(1))
            .order_by_asc(Column::Key)
            .into_tuple::<String>()
            .all(&self.orm)
            .await
            .map_err(database_error)
    }
}

/// Borrowed form of the columns a secret write needs.
pub struct StoredWrite<'a> {
    /// The stored representation bytes.
    pub value: &'a [u8],
    /// The owning purpose, absent for a legacy plaintext write.
    pub purpose: Option<&'a str>,
    /// Master-key version.
    pub key_version: i32,
    /// Representation marker.
    pub ciphertext_version: i32,
}

impl<'a> From<&'a StoredSecretBytes> for StoredWrite<'a> {
    fn from(bytes: &'a StoredSecretBytes) -> Self {
        Self {
            value: &bytes.value,
            purpose: bytes.purpose.as_deref(),
            key_version: bytes.key_version,
            ciphertext_version: bytes.ciphertext_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::database_error;
    use sea_orm::{DbErr, RuntimeErr};
    use std::sync::Arc;

    #[test]
    fn extracts_owned_driver_error() {
        let error = DbErr::Query(RuntimeErr::SqlxError(Arc::new(sqlx::Error::RowNotFound)));
        assert!(matches!(database_error(error), sqlx::Error::RowNotFound));
    }

    #[test]
    fn shared_driver_error_uses_protocol_fallback() {
        let shared = Arc::new(sqlx::Error::RowNotFound);
        let error = DbErr::Query(RuntimeErr::SqlxError(Arc::clone(&shared)));
        assert!(matches!(database_error(error), sqlx::Error::Protocol(_)));
    }
}
