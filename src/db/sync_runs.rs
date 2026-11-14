use anyhow::Result;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, InsertStatement, OnConflict, Query};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use super::entity::{source_checkpoints, source_configs, sync_runs};
use super::{Database, RunHandle, SyncCheckpoint, SyncOutcome};
use crate::contracts::Visibility;

fn start_run_query(source_key: &str, trigger_type: &str) -> Result<InsertStatement> {
    let select = Query::select()
        .columns([
            source_configs::Column::GroupId,
            source_configs::Column::Visibility,
            source_configs::Column::SourceKey,
        ])
        .expr(Expr::value(trigger_type.to_owned()))
        .expr(Expr::value("running"))
        .from((Alias::new("context69"), source_configs::Entity))
        .and_where(Expr::col(source_configs::Column::SourceKey).eq(source_key))
        .to_owned();
    Ok(Query::insert()
        .into_table((Alias::new("context69"), sync_runs::Entity))
        .columns([
            sync_runs::Column::GroupId,
            sync_runs::Column::Visibility,
            sync_runs::Column::SourceKey,
            sync_runs::Column::TriggerType,
            sync_runs::Column::Status,
        ])
        .select_from(select)?
        .returning_col(sync_runs::Column::Id)
        .to_owned())
}

fn checkpoint_query(source_key: &str, checkpoint: &SyncCheckpoint) -> Result<InsertStatement> {
    let select = Query::select()
        .columns([
            source_configs::Column::GroupId,
            source_configs::Column::Visibility,
            source_configs::Column::SourceKey,
        ])
        .expr(Expr::value(checkpoint.updated_at))
        .expr(Expr::value(checkpoint.external_id.clone()))
        .expr(Expr::current_timestamp())
        .expr(Expr::current_timestamp())
        .from((Alias::new("context69"), source_configs::Entity))
        .and_where(Expr::col(source_configs::Column::SourceKey).eq(source_key))
        .to_owned();
    Ok(Query::insert()
        .into_table((Alias::new("context69"), source_checkpoints::Entity))
        .columns([
            source_checkpoints::Column::GroupId,
            source_checkpoints::Column::Visibility,
            source_checkpoints::Column::SourceKey,
            source_checkpoints::Column::CursorUpdatedAt,
            source_checkpoints::Column::CursorExternalId,
            source_checkpoints::Column::LastSuccessAt,
            source_checkpoints::Column::UpdatedAt,
        ])
        .select_from(select)?
        .on_conflict(
            OnConflict::column(source_checkpoints::Column::SourceKey)
                .update_columns([
                    source_checkpoints::Column::CursorUpdatedAt,
                    source_checkpoints::Column::CursorExternalId,
                ])
                .value(
                    source_checkpoints::Column::LastSuccessAt,
                    Expr::current_timestamp(),
                )
                .value(
                    source_checkpoints::Column::UpdatedAt,
                    Expr::current_timestamp(),
                )
                .to_owned(),
        )
        .to_owned())
}

fn sync_database_error(error: sea_orm::DbErr) -> anyhow::Error {
    use sea_orm::{DbErr, RuntimeErr};
    let (error, wrap): (_, fn(RuntimeErr) -> DbErr) = match error {
        DbErr::Exec(RuntimeErr::SqlxError(error)) => (error, DbErr::Exec),
        DbErr::Query(RuntimeErr::SqlxError(error)) => (error, DbErr::Query),
        DbErr::Conn(RuntimeErr::SqlxError(error)) => (error, DbErr::Conn),
        error => return error.into(),
    };
    match std::sync::Arc::try_unwrap(error) {
        Ok(error) => error.into(),
        Err(error) => wrap(RuntimeErr::SqlxError(error)).into(),
    }
}

impl Database {
    pub async fn start_run(&self, source_key: &str, trigger_type: &str) -> Result<RunHandle> {
        let statement = start_run_query(source_key, trigger_type)?;
        let row = self
            .orm
            .query_one(&statement)
            .await
            .map_err(sync_database_error)?
            .ok_or(sqlx::Error::RowNotFound)?;
        let id = row.try_get::<i64>("", "id")?;

        Ok(RunHandle {
            id,
            source_key: source_key.to_string(),
        })
    }

    pub async fn finish_run(
        &self,
        run: &RunHandle,
        status: &str,
        outcome: &SyncOutcome,
        error_message: Option<&str>,
    ) -> Result<()> {
        sync_runs::Entity::update_many()
            .col_expr(sync_runs::Column::Status, Expr::value(status.to_owned()))
            .col_expr(
                sync_runs::Column::RecordsSeen,
                Expr::value(outcome.records_seen as i32),
            )
            .col_expr(
                sync_runs::Column::RecordsChanged,
                Expr::value(outcome.records_changed as i32),
            )
            .col_expr(
                sync_runs::Column::ChunksUpserted,
                Expr::value(outcome.chunks_upserted as i32),
            )
            .col_expr(
                sync_runs::Column::ErrorMessage,
                Expr::value(error_message.map(str::to_owned)),
            )
            .col_expr(sync_runs::Column::FinishedAt, Expr::current_timestamp())
            .col_expr(sync_runs::Column::UpdatedAt, Expr::current_timestamp())
            .filter(sync_runs::Column::Id.eq(run.id))
            .exec(&self.orm)
            .await
            .map_err(sync_database_error)?;
        Ok(())
    }

    /// The run of one scoped sync, started on a borrowed connection.
    pub async fn start_run_in_scope_in(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        visibility: Visibility,
        source_key: &str,
        trigger_type: &str,
    ) -> Result<RunHandle> {
        let id = sqlx::query_file_scalar!(
            "src/sql/db/sync_runs/start_run_in_scope.sql",
            group_id,
            visibility.as_str(),
            source_key,
            trigger_type
        )
        .fetch_one(&mut *connection)
        .await?;

        Ok(RunHandle {
            id,
            source_key: source_key.to_string(),
        })
    }

    /// The outcome of one run, recorded on a borrowed connection.
    pub async fn finish_run_in(
        &self,
        connection: &mut sqlx::PgConnection,
        run: &RunHandle,
        status: &str,
        outcome: &SyncOutcome,
        error_message: Option<&str>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/sync_runs/finish_run.sql",
            run.id,
            status,
            outcome.records_seen as i32,
            outcome.records_changed as i32,
            outcome.chunks_upserted as i32,
            error_message
        )
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// The checkpoint of one scoped sync, saved on a borrowed connection.
    pub async fn save_checkpoint_in_scope_in(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        visibility: Visibility,
        source_key: &str,
        checkpoint: &SyncCheckpoint,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/sync_runs/save_checkpoint_in_scope.sql",
            group_id,
            visibility.as_str(),
            source_key,
            checkpoint.updated_at,
            checkpoint.external_id.as_deref()
        )
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// The sync state of one project, removed on a borrowed connection.
    pub async fn delete_sync_state_in_project_in(
        &self,
        connection: &mut sqlx::PgConnection,
        project_id: i64,
        source_key: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/sync_runs/delete_checkpoint_in_project.sql",
            project_id,
            source_key
        )
        .execute(&mut *connection)
        .await?;
        sqlx::query_file!(
            "src/sql/db/sync_runs/delete_runs_in_project.sql",
            project_id,
            source_key
        )
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    pub async fn get_checkpoint(&self, source_key: &str) -> Result<SyncCheckpoint> {
        let row = source_checkpoints::Entity::find_by_id(source_key.to_owned())
            .one(&self.orm)
            .await
            .map_err(sync_database_error)?;

        Ok(row
            .map(|row| SyncCheckpoint {
                updated_at: row.cursor_updated_at,
                external_id: row.cursor_external_id,
            })
            .unwrap_or(SyncCheckpoint {
                updated_at: None,
                external_id: None,
            }))
    }

    pub async fn save_checkpoint(
        &self,
        source_key: &str,
        checkpoint: &SyncCheckpoint,
    ) -> Result<()> {
        let statement = checkpoint_query(source_key, checkpoint)?;
        self.orm
            .execute(&statement)
            .await
            .map_err(sync_database_error)?;
        Ok(())
    }

    pub async fn delete_sync_state_in_project(
        &self,
        project_id: i64,
        source_key: &str,
    ) -> Result<()> {
        source_checkpoints::Entity::delete_many()
            .filter(source_checkpoints::Column::GroupId.eq(project_id))
            .filter(source_checkpoints::Column::SourceKey.eq(source_key))
            .exec(&self.orm)
            .await
            .map_err(sync_database_error)?;
        sync_runs::Entity::delete_many()
            .filter(sync_runs::Column::GroupId.eq(project_id))
            .filter(sync_runs::Column::SourceKey.eq(source_key))
            .exec(&self.orm)
            .await
            .map_err(sync_database_error)?;
        Ok(())
    }

    pub async fn rename_sync_state_in_project(
        &self,
        project_id: i64,
        old_source_key: &str,
        new_source_key: &str,
    ) -> Result<()> {
        if old_source_key == new_source_key {
            return Ok(());
        }
        source_checkpoints::Entity::update_many()
            .col_expr(
                source_checkpoints::Column::SourceKey,
                Expr::value(new_source_key.to_owned()),
            )
            .col_expr(
                source_checkpoints::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .filter(source_checkpoints::Column::GroupId.eq(project_id))
            .filter(source_checkpoints::Column::SourceKey.eq(old_source_key))
            .exec(&self.orm)
            .await
            .map_err(sync_database_error)?;
        sync_runs::Entity::update_many()
            .col_expr(
                sync_runs::Column::SourceKey,
                Expr::value(new_source_key.to_owned()),
            )
            .col_expr(sync_runs::Column::UpdatedAt, Expr::current_timestamp())
            .filter(sync_runs::Column::GroupId.eq(project_id))
            .filter(sync_runs::Column::SourceKey.eq(old_source_key))
            .exec(&self.orm)
            .await
            .map_err(sync_database_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncCheckpoint, checkpoint_query, start_run_query};
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn sync_inserts_use_source_scope_in_same_statement() {
        let start = start_run_query("source", "manual")
            .unwrap()
            .to_string(PostgresQueryBuilder);
        assert!(
            start.contains("FROM \"context69\".\"source_configs\" WHERE \"source_key\" = 'source'"),
            "{start}"
        );
        assert!(start.contains("RETURNING \"id\""), "{start}");
        let checkpoint = checkpoint_query(
            "source",
            &SyncCheckpoint {
                updated_at: None,
                external_id: None,
            },
        )
        .unwrap()
        .to_string(PostgresQueryBuilder);
        assert!(
            checkpoint
                .contains("FROM \"context69\".\"source_configs\" WHERE \"source_key\" = 'source'"),
            "{checkpoint}"
        );
        assert!(
            checkpoint.contains("ON CONFLICT (\"source_key\") DO UPDATE"),
            "{checkpoint}"
        );
        let update = checkpoint.split("DO UPDATE SET").nth(1).unwrap();
        assert!(!update.contains("\"group_id\""), "{checkpoint}");
        assert!(!update.contains("\"visibility\""), "{checkpoint}");
    }
}
