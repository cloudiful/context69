use crate::entity::document_translation_jobs::{Column, Entity};
use anyhow::{Result, bail};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, Func, Query};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{TranslationJobRecord, TranslationStore, TranslationVersionInput};

pub(crate) struct FinishJob<'a> {
    pub id: Uuid,
    pub status: &'a str,
    pub source_locale: Option<&'a str>,
    pub provider_key: Option<&'a str>,
    pub provider_config_hash: Option<&'a str>,
    pub character_count: i64,
    pub error_message: Option<&'a str>,
}

pub(crate) struct TranslationAttempt<'a> {
    pub job_id: Uuid,
    pub provider_key: &'a str,
    pub provider_config_hash: &'a str,
    pub attempt_number: i32,
    pub status: &'a str,
    pub character_count: i64,
    pub latency_ms: i64,
    pub error_message: Option<&'a str>,
}

fn document_in_group(group_id: i64) -> Expr {
    Expr::exists(
        Query::select()
            .expr(Expr::value(1))
            .from((Alias::new("context69"), Alias::new("documents")))
            .and_where(
                Expr::col((Alias::new("documents"), Alias::new("id")))
                    .equals((Entity, Column::DocumentId)),
            )
            .and_where(Expr::col((Alias::new("documents"), Alias::new("group_id"))).eq(group_id))
            .to_owned(),
    )
}

impl TranslationStore {
    pub async fn job_in_group(
        &self,
        group_id: i64,
        id: Uuid,
    ) -> Result<Option<TranslationJobRecord>> {
        Ok(Entity::find()
            .filter(Column::Id.eq(id))
            .filter(document_in_group(group_id))
            .one(&self.orm)
            .await?
            .map(Into::into))
    }

    pub async fn jobs_for_document(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<Vec<TranslationJobRecord>> {
        Ok(Entity::find()
            .filter(Column::DocumentId.eq(document_id))
            .filter(document_in_group(group_id))
            .order_by_desc(Column::CreatedAt)
            .order_by_desc(Column::Id)
            .all(&self.orm)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn retry_job(&self, group_id: i64, id: Uuid) -> Result<Option<TranslationJobRecord>> {
        let rows = Entity::update_many()
            .col_expr(Column::Status, Expr::value("queued"))
            .col_expr(Column::ErrorMessage, Expr::value(None::<String>))
            .col_expr(
                Column::FinishedAt,
                Expr::value(None::<chrono::DateTime<chrono::Utc>>),
            )
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Id.eq(id))
            .filter(Column::Status.is_in(["failed", "quota_exceeded"]))
            .filter(document_in_group(group_id))
            .exec_with_returning(&self.orm)
            .await?;
        Ok(rows.into_iter().next().map(Into::into))
    }

    pub(crate) async fn finish_job(&self, result: FinishJob<'_>) -> Result<TranslationJobRecord> {
        let rows = Entity::update_many()
            .col_expr(Column::Status, Expr::value(result.status))
            .col_expr(
                Column::DetectedSourceLocale,
                Expr::value(result.source_locale),
            )
            .col_expr(Column::ProviderKey, Expr::value(result.provider_key))
            .col_expr(
                Column::ProviderConfigHash,
                Expr::value(result.provider_config_hash),
            )
            .col_expr(
                Column::SourceCharacterCount,
                Expr::value(result.character_count),
            )
            .col_expr(Column::ErrorMessage, Expr::value(result.error_message))
            .col_expr(Column::FinishedAt, Expr::current_timestamp())
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Id.eq(result.id))
            .exec_with_returning(&self.orm)
            .await?;
        Ok(rows
            .into_iter()
            .next()
            .ok_or(sqlx::Error::RowNotFound)?
            .into())
    }

    pub(crate) async fn release_claimed_job(&self, id: Uuid) -> Result<bool> {
        let result = Entity::update_many()
            .col_expr(Column::Status, Expr::value("queued"))
            .col_expr(
                Column::AttemptCount,
                Func::greatest([Expr::col(Column::AttemptCount).sub(1), Expr::value(0)]).into(),
            )
            .col_expr(
                Column::StartedAt,
                Expr::value(None::<chrono::DateTime<chrono::Utc>>),
            )
            .col_expr(
                Column::FinishedAt,
                Expr::value(None::<chrono::DateTime<chrono::Utc>>),
            )
            .col_expr(Column::ErrorMessage, Expr::value(None::<String>))
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Id.eq(id))
            .filter(Column::Status.eq("running"))
            .exec(&self.orm)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn insert_attempt(&self, attempt: TranslationAttempt<'_>) -> Result<()> {
        use crate::entity::document_translation_attempts::{ActiveModel, Entity};
        use sea_orm::ActiveValue::Set;
        Entity::insert(ActiveModel {
            job_id: Set(attempt.job_id),
            provider_key: Set(attempt.provider_key.to_owned()),
            provider_config_hash: Set(attempt.provider_config_hash.to_owned()),
            attempt_number: Set(attempt.attempt_number),
            status: Set(attempt.status.to_owned()),
            character_count: Set(attempt.character_count),
            latency_ms: Set(attempt.latency_ms),
            error_message: Set(attempt.error_message.map(str::to_owned)),
            ..Default::default()
        })
        .exec(&self.orm)
        .await?;
        Ok(())
    }

    pub async fn current_translation_chunk_ids(
        &self,
        document_id: i64,
        target_locale: &str,
    ) -> Result<Vec<Uuid>> {
        let mut connection = self.pool().acquire().await?;
        self.current_translation_chunk_ids_in(&mut connection, document_id, target_locale)
            .await
    }

    /// The ids of the translation chunks that are live for this document under
    /// the caller's transaction.
    ///
    /// The statement joins the current document record hash, so a caller that
    /// holds the document row lock reads exactly the live set it is about to
    /// replace.
    pub async fn current_translation_chunk_ids_in(
        &self,
        connection: &mut PgConnection,
        document_id: i64,
        target_locale: &str,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "sql/versions/list_chunk_ids.sql",
            document_id,
            target_locale
        )
        .fetch_all(&mut *connection)
        .await?)
    }

    /// Every stored translation chunk id of a document on the caller's
    /// connection.
    ///
    /// The result holds the ids of every stored version, the current one and
    /// the stale ones. A target locale selects its own set and `None` selects
    /// every locale. A registry-aware caller uses it to reserve and retire the
    /// complete set a locale owns; the shared crate itself knows nothing about
    /// the registry.
    pub async fn all_translation_chunk_ids_in(
        &self,
        connection: &mut PgConnection,
        document_id: i64,
        target_locale: Option<&str>,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "sql/versions/list_all_chunk_ids.sql",
            document_id,
            target_locale
        )
        .fetch_all(&mut *connection)
        .await?)
    }

    /// The current source record hash of a document, locked for the caller's
    /// transaction. A publication rechecks this under its commit lock so a
    /// provider round trip cannot publish a version of a moved document.
    pub async fn lock_document_hash_in(
        &self,
        connection: &mut PgConnection,
        document_id: i64,
    ) -> Result<String> {
        Ok(
            sqlx::query_file_scalar!("sql/versions/lock_document_hash.sql", document_id)
                .fetch_one(connection)
                .await?,
        )
    }

    /// Writes a translation version and its chunks on the caller's connection.
    ///
    /// The current source record hash is locked and compared first: a document
    /// that moved after the provider call returns `false` without deleting or
    /// inserting anything, so the caller can refuse the publication inside its
    /// own transaction.
    pub async fn publish_version_in(
        &self,
        connection: &mut PgConnection,
        version: &TranslationVersionInput<'_>,
        chunks: &[(Uuid, String)],
    ) -> Result<bool> {
        let current = self
            .lock_document_hash_in(&mut *connection, version.document_id)
            .await?;
        if current != version.source_record_hash {
            return Ok(false);
        }
        sqlx::query_file!(
            "sql/versions/delete_current.sql",
            version.document_id,
            version.target_locale,
            version.source_record_hash
        )
        .execute(&mut *connection)
        .await?;
        sqlx::query_file!(
            "sql/versions/insert.sql",
            version.id,
            version.document_id,
            version.target_locale,
            version.source_locale,
            version.source_record_hash,
            version.provider_key,
            version.provider_config_hash,
            version.model_name,
            version.title,
            version.summary,
            version.body_text
        )
        .execute(&mut *connection)
        .await?;
        for (index, (id, text)) in chunks.iter().enumerate() {
            sqlx::query_file!(
                "sql/versions/insert_chunk.sql",
                id,
                version.id,
                version.document_id,
                version.target_locale,
                index as i32,
                text
            )
            .execute(&mut *connection)
            .await?;
        }
        Ok(true)
    }

    pub async fn publish_version(
        &self,
        version: &TranslationVersionInput<'_>,
        chunks: &[(Uuid, String)],
    ) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        if !self.publish_version_in(&mut tx, version, chunks).await? {
            tx.rollback().await?;
            bail!("translation source document changed");
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    #[test]
    fn retry_scope_is_correlated_to_the_document() {
        let query = Entity::update_many()
            .filter(document_in_group(7))
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            query.contains("\"documents\".\"id\" = \"document_translation_jobs\".\"document_id\""),
            "{query}"
        );
        assert!(query.contains("\"documents\".\"group_id\" = 7"), "{query}");
    }
}
