use crate::db::entity::{groups, source_checkpoints, source_configs};
use anyhow::{Result, anyhow};
use sea_orm::sea_query::{
    Alias, Expr, ExprTrait, Func, InsertStatement, JoinType, OnConflict, Order, Query,
    SelectStatement, extension::postgres::PgExpr,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, SqlErr, TransactionTrait,
};
use sqlx::types::Json;
use uuid::Uuid;

use super::{
    SourceConfig, SourceConfigRow, SourceStatus, SourceStatusRow, SourceStore,
    row_to_source_config, row_to_source_status,
};

fn status_query() -> SelectStatement {
    let sc = Alias::new("sc");
    let g = Alias::new("g");
    let cp = Alias::new("cp");
    Query::select()
        .columns([
            (sc.clone(), source_configs::Column::GroupId),
            (sc.clone(), source_configs::Column::Visibility),
            (sc.clone(), source_configs::Column::SourceKey),
            (sc.clone(), source_configs::Column::DisplayName),
            (sc.clone(), source_configs::Column::Description),
            (sc.clone(), source_configs::Column::ExampleQueries),
            (sc.clone(), source_configs::Column::Connection),
            (sc.clone(), source_configs::Column::SyncStrategy),
            (sc.clone(), source_configs::Column::BaseQuery),
            (sc.clone(), source_configs::Column::BatchSize),
        ])
        .column((g.clone(), groups::Column::GroupKey))
        .expr_as(
            Expr::col((g.clone(), groups::Column::FullPath)),
            Alias::new("group_path"),
        )
        .expr_as(
            Expr::col((cp.clone(), source_checkpoints::Column::CursorUpdatedAt)),
            Alias::new("last_cursor_updated_at"),
        )
        .expr_as(
            Expr::col((cp.clone(), source_checkpoints::Column::CursorExternalId)),
            Alias::new("last_cursor_external_id"),
        )
        .column((cp.clone(), source_checkpoints::Column::LastSuccessAt))
        .from_as(
            (Alias::new("context69"), source_configs::Entity),
            sc.clone(),
        )
        .join_as(
            JoinType::InnerJoin,
            (Alias::new("context69"), groups::Entity),
            g.clone(),
            Expr::col((g, groups::Column::Id))
                .equals((sc.clone(), source_configs::Column::GroupId)),
        )
        .join_as(
            JoinType::LeftJoin,
            (Alias::new("context69"), source_checkpoints::Entity),
            cp.clone(),
            Expr::col((cp, source_checkpoints::Column::SourceKey))
                .equals((sc, source_configs::Column::SourceKey)),
        )
        .to_owned()
}

fn example_queries(value: serde_json::Value) -> Result<Json<Vec<String>>> {
    serde_json::from_value(value).map(Json).map_err(|error| {
        sqlx::Error::ColumnDecode {
            index: "example_queries".to_owned(),
            source: Box::new(error),
        }
        .into()
    })
}

fn status_row(row: sea_orm::QueryResult) -> Result<SourceStatusRow> {
    Ok(SourceStatusRow {
        group_id: row.try_get("", "group_id")?,
        group_key: row.try_get("", "group_key")?,
        group_path: row.try_get("", "group_path")?,
        visibility: row.try_get("", "visibility")?,
        source_key: row.try_get("", "source_key")?,
        display_name: row.try_get("", "display_name")?,
        description: row.try_get("", "description")?,
        example_queries: example_queries(row.try_get("", "example_queries")?)?,
        connection: row.try_get("", "connection")?,
        sync_strategy: row.try_get("", "sync_strategy")?,
        base_query: row.try_get("", "base_query")?,
        batch_size: row.try_get("", "batch_size")?,
        last_cursor_updated_at: row.try_get("", "last_cursor_updated_at")?,
        last_cursor_external_id: row.try_get("", "last_cursor_external_id")?,
        last_success_at: row.try_get("", "last_success_at")?,
    })
}

fn default_scope_insert(source: &SourceConfig, ignore_conflict: bool) -> Result<InsertStatement> {
    let select = Query::select()
        .column(Alias::new("id"))
        .expr(Expr::value("public"))
        .expr(Expr::value(source.key.clone()))
        .expr(Expr::value(source.display_name.clone()))
        .expr(Expr::value(source.description.clone()))
        .expr(Expr::value(serde_json::to_value(&source.example_queries)?))
        .expr(Expr::value(source.connection.clone()))
        .expr(Expr::value(source.sync_strategy.as_str().to_owned()))
        .expr(Expr::value(source.connector_type().to_owned()))
        .expr(Expr::value(source.connector.base_query.clone()))
        .expr(Expr::value(source.connector.batch_size))
        .from((Alias::new("context69"), Alias::new("groups")))
        .and_where(Expr::col(Alias::new("full_path")).eq("public"))
        .to_owned();
    let mut insert = Query::insert();
    insert
        .into_table((Alias::new("context69"), source_configs::Entity))
        .columns([
            source_configs::Column::GroupId,
            source_configs::Column::Visibility,
            source_configs::Column::SourceKey,
            source_configs::Column::DisplayName,
            source_configs::Column::Description,
            source_configs::Column::ExampleQueries,
            source_configs::Column::Connection,
            source_configs::Column::SyncStrategy,
            source_configs::Column::ConnectorType,
            source_configs::Column::BaseQuery,
            source_configs::Column::BatchSize,
        ])
        .select_from(select)?;
    if ignore_conflict {
        insert.on_conflict(
            OnConflict::column(source_configs::Column::SourceKey)
                .do_nothing()
                .to_owned(),
        );
    }
    Ok(insert)
}

fn source_database_error(error: sea_orm::DbErr) -> anyhow::Error {
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

impl SourceStore {
    pub async fn seed_sources_if_empty(&self, sources: &[SourceConfig]) -> Result<()> {
        let count = source_configs::Entity::find()
            .count(self.db.orm())
            .await
            .map_err(source_database_error)?;

        if count > 0 || sources.is_empty() {
            return Ok(());
        }

        let tx = self.db.orm().begin().await.map_err(source_database_error)?;
        for source in sources {
            tx.execute(&default_scope_insert(source, true)?)
                .await
                .map_err(source_database_error)?;
        }
        tx.commit().await.map_err(source_database_error)?;
        Ok(())
    }

    pub async fn list_source_configs(&self) -> Result<Vec<SourceConfig>> {
        let rows = source_configs::Entity::find()
            .order_by_asc(source_configs::Column::SourceKey)
            .all(self.db.orm())
            .await
            .map_err(source_database_error)?;
        rows.into_iter()
            .map(|row| {
                row_to_source_config(SourceConfigRow {
                    source_key: row.source_key,
                    display_name: row.display_name,
                    description: row.description,
                    example_queries: example_queries(row.example_queries)?,
                    connection: row.connection,
                    sync_strategy: row.sync_strategy,
                    connector_type: row.connector_type,
                    base_query: row.base_query,
                    batch_size: row.batch_size,
                })
            })
            .collect()
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>> {
        let mut query = status_query();
        query.order_by(
            (Alias::new("sc"), source_configs::Column::SourceKey),
            Order::Asc,
        );
        self.db
            .orm()
            .query_all(&query)
            .await
            .map_err(source_database_error)?
            .into_iter()
            .map(|row| Ok(row_to_source_status(status_row(row)?)))
            .collect()
    }

    pub async fn count_sources(&self, query: Option<&str>) -> Result<i64> {
        let mut selection = source_configs::Entity::find();
        if let Some(query) = query {
            let pattern = format!("%{query}%");
            selection = selection.filter(
                Condition::any()
                    .add(source_configs::Column::SourceKey.ilike(pattern.clone()))
                    .add(
                        Expr::expr(Func::coalesce([
                            Expr::col(source_configs::Column::DisplayName),
                            Expr::value(""),
                        ]))
                        .ilike(pattern.clone()),
                    )
                    .add(
                        Expr::expr(Func::coalesce([
                            Expr::col(source_configs::Column::Description),
                            Expr::value(""),
                        ]))
                        .ilike(pattern),
                    ),
            );
        }
        Ok(i64::try_from(
            selection
                .count(self.db.orm())
                .await
                .map_err(source_database_error)?,
        )?)
    }

    pub async fn list_sources_page(
        &self,
        query: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SourceStatus>> {
        let (Ok(limit), Ok(offset)) = (u64::try_from(limit), u64::try_from(offset)) else {
            let rows = sqlx::query_file_as!(
                SourceStatusRow,
                "src/sql/source_store/list_sources_page.sql",
                query,
                limit,
                offset
            )
            .fetch_all(self.db.pool())
            .await?;
            return Ok(rows.into_iter().map(row_to_source_status).collect());
        };
        let mut statement = status_query();
        let sc = Alias::new("sc");
        if let Some(query) = query {
            let pattern = format!("%{query}%");
            statement.cond_where(
                Condition::any()
                    .add(
                        Expr::col((sc.clone(), source_configs::Column::SourceKey))
                            .ilike(pattern.clone()),
                    )
                    .add(
                        Expr::expr(Func::coalesce([
                            Expr::col((sc.clone(), source_configs::Column::DisplayName)),
                            Expr::value(""),
                        ]))
                        .ilike(pattern.clone()),
                    )
                    .add(
                        Expr::expr(Func::coalesce([
                            Expr::col((sc.clone(), source_configs::Column::Description)),
                            Expr::value(""),
                        ]))
                        .ilike(pattern),
                    ),
            );
        }
        statement
            .order_by((sc, source_configs::Column::SourceKey), Order::Asc)
            .limit(limit)
            .offset(offset);
        self.db
            .orm()
            .query_all(&statement)
            .await
            .map_err(source_database_error)?
            .into_iter()
            .map(|row| Ok(row_to_source_status(status_row(row)?)))
            .collect()
    }

    pub async fn list_sources_for_group(&self, group_id: i64) -> Result<Vec<SourceStatus>> {
        let mut statement = status_query();
        let sc = Alias::new("sc");
        statement
            .and_where(Expr::col((sc.clone(), source_configs::Column::GroupId)).eq(group_id))
            .order_by((sc, source_configs::Column::SourceKey), Order::Asc);
        self.db
            .orm()
            .query_all(&statement)
            .await
            .map_err(source_database_error)?
            .into_iter()
            .map(|row| Ok(row_to_source_status(status_row(row)?)))
            .collect()
    }

    pub async fn get_source(&self, source_key: &str) -> Result<Option<SourceStatus>> {
        let mut statement = status_query();
        statement.and_where(
            Expr::col((Alias::new("sc"), source_configs::Column::SourceKey)).eq(source_key),
        );
        self.db
            .orm()
            .query_one(&statement)
            .await
            .map_err(source_database_error)?
            .map(|row| Ok(row_to_source_status(status_row(row)?)))
            .transpose()
    }

    pub async fn get_source_in_group(
        &self,
        group_id: i64,
        source_key: &str,
    ) -> Result<Option<SourceStatus>> {
        let mut statement = status_query();
        let sc = Alias::new("sc");
        statement
            .and_where(Expr::col((sc.clone(), source_configs::Column::GroupId)).eq(group_id))
            .and_where(Expr::col((sc, source_configs::Column::SourceKey)).eq(source_key));
        self.db
            .orm()
            .query_one(&statement)
            .await
            .map_err(source_database_error)?
            .map(|row| Ok(row_to_source_status(status_row(row)?)))
            .transpose()
    }

    pub async fn get_source_scope(&self, source_key: &str) -> Result<Option<super::SourceScope>> {
        let mut statement = status_query();
        statement.and_where(
            Expr::col((Alias::new("sc"), source_configs::Column::SourceKey)).eq(source_key),
        );
        let row = self
            .db
            .orm()
            .query_one(&statement)
            .await
            .map_err(source_database_error)?
            .map(status_row)
            .transpose()?;
        Ok(row.map(|row| super::SourceScope {
            group_id: row.group_id,
            group_key: row.group_key,
            group_path: row.group_path,
            visibility: row
                .visibility
                .parse()
                .unwrap_or(crate::contracts::Visibility::Private),
        }))
    }

    pub async fn insert_source(&self, source: &SourceConfig) -> Result<()> {
        let result = self
            .db
            .orm()
            .execute(&default_scope_insert(source, false)?)
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                Err(anyhow!("source {} already exists", source.key))
            }
            Err(error) => Err(source_database_error(error)),
        }
    }

    pub async fn insert_source_in_scope(
        &self,
        source: &SourceConfig,
        scope: &super::SourceScope,
    ) -> Result<()> {
        let result = source_configs::Entity::insert(source_configs::ActiveModel {
            group_id: Set(scope.group_id),
            visibility: Set(scope.visibility.as_str().to_owned()),
            source_key: Set(source.key.clone()),
            display_name: Set(source.display_name.clone()),
            description: Set(source.description.clone()),
            example_queries: Set(serde_json::to_value(&source.example_queries)?),
            connection: Set(source.connection.clone()),
            sync_strategy: Set(source.sync_strategy.as_str().to_owned()),
            connector_type: Set(source.connector_type().to_owned()),
            base_query: Set(source.connector.base_query.clone()),
            batch_size: Set(source.connector.batch_size),
            ..Default::default()
        })
        .exec(self.db.orm())
        .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                Err(anyhow!("source {} already exists", source.key))
            }
            Err(error) => Err(source_database_error(error)),
        }
    }

    pub async fn update_source(&self, source_key: &str, source: &SourceConfig) -> Result<()> {
        self.update_source_in_group(None, source_key, source).await
    }

    pub async fn update_source_in_group(
        &self,
        group_id: Option<i64>,
        source_key: &str,
        source: &SourceConfig,
    ) -> Result<()> {
        let mut update = source_configs::Entity::update_many()
            .col_expr(
                source_configs::Column::DisplayName,
                Expr::value(source.display_name.clone()),
            )
            .col_expr(
                source_configs::Column::Description,
                Expr::value(source.description.clone()),
            )
            .col_expr(
                source_configs::Column::ExampleQueries,
                Expr::value(serde_json::to_value(&source.example_queries)?),
            )
            .col_expr(
                source_configs::Column::Connection,
                Expr::value(source.connection.clone()),
            )
            .col_expr(
                source_configs::Column::SyncStrategy,
                Expr::value(source.sync_strategy.as_str().to_owned()),
            )
            .col_expr(
                source_configs::Column::ConnectorType,
                Expr::value(source.connector_type().to_owned()),
            )
            .col_expr(
                source_configs::Column::BaseQuery,
                Expr::value(source.connector.base_query.clone()),
            )
            .col_expr(
                source_configs::Column::BatchSize,
                Expr::value(source.connector.batch_size),
            )
            .col_expr(source_configs::Column::UpdatedAt, Expr::current_timestamp())
            .filter(source_configs::Column::SourceKey.eq(source_key));
        if let Some(group_id) = group_id {
            update = update.filter(source_configs::Column::GroupId.eq(group_id));
        }
        let result = update
            .exec(self.db.orm())
            .await
            .map_err(source_database_error)?;

        if result.rows_affected == 0 {
            return Err(anyhow!("unknown source {source_key}"));
        }

        Ok(())
    }

    /// The exact original and translated chunk ids of one source, read on a
    /// caller connection so the remote purge can name what the canonical
    /// delete is about to remove.
    pub(crate) async fn list_source_chunk_ids_on(
        &self,
        connection: &mut sqlx::PgConnection,
        source_key: &str,
    ) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/source_store/list_source_chunk_ids.sql", source_key)
                .fetch_all(connection)
                .await?,
        )
    }

    /// The document ids the canonical source delete owns.
    ///
    /// The predicate is the same source_key/group scope as the delete, so a
    /// tombstone never names a document outside the deleted source.
    pub(crate) async fn list_source_document_ids_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: Option<i64>,
        source_key: &str,
    ) -> Result<Vec<i64>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/source_store/list_source_document_ids.sql",
            source_key,
            group_id
        )
        .fetch_all(connection)
        .await?)
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<bool> {
        self.delete_source_in_group(None, source_key).await
    }

    pub async fn delete_source_in_group(
        &self,
        group_id: Option<i64>,
        source_key: &str,
    ) -> Result<bool> {
        let exists = self.source_exists_in_group(group_id, source_key).await?;

        if !exists {
            return Ok(false);
        }

        let mut tx = self.db.pool().begin().await?;
        delete_source_rows_on(&mut tx, group_id, source_key).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Whether one group scope holds the source.
    pub async fn source_exists_in_group(
        &self,
        group_id: Option<i64>,
        source_key: &str,
    ) -> Result<bool> {
        let mut selection = source_configs::Entity::find_by_id(source_key.to_owned());
        if let Some(group_id) = group_id {
            selection = selection.filter(source_configs::Column::GroupId.eq(group_id));
        }
        Ok(selection
            .one(self.db.orm())
            .await
            .map_err(source_database_error)?
            .is_some())
    }

    /// Removes the legacy source and everything it owns on a borrowed
    /// connection.
    ///
    /// A task-owned delete runs the four statements in one guarded transaction
    /// of its captured grant, so a worker that lost its segment removes nothing
    /// and a partial failure leaves no half-deleted source.
    pub async fn delete_source_in_group_in(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: Option<i64>,
        source_key: &str,
    ) -> Result<bool> {
        let exists = sqlx::query_file_scalar!(
            "src/sql/source_store/source_exists_in_project.sql",
            source_key,
            group_id
        )
        .fetch_one(&mut *connection)
        .await?;

        if !exists {
            return Ok(false);
        }

        delete_source_rows_on(connection, group_id, source_key).await?;
        Ok(true)
    }
}

/// The four statements that remove one legacy source and everything it owns.
async fn delete_source_rows_on(
    connection: &mut sqlx::PgConnection,
    group_id: Option<i64>,
    source_key: &str,
) -> Result<()> {
    sqlx::query_file!(
        "src/sql/source_store/delete_source_sync_runs.sql",
        source_key,
        group_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "src/sql/source_store/delete_source_checkpoints.sql",
        source_key,
        group_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "src/sql/source_store/delete_source_documents.sql",
        source_key,
        group_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "src/sql/source_store/delete_source_configs.sql",
        source_key,
        group_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PostgresSqlConnectorConfig, SyncStrategy};
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn default_scope_insert_stays_single_statement() {
        let source = SourceConfig {
            key: "example".into(),
            display_name: None,
            description: None,
            example_queries: vec![],
            connection: "example".into(),
            sync_strategy: SyncStrategy::Cursor,
            connector: PostgresSqlConnectorConfig {
                base_query: "select 1".into(),
                batch_size: 1,
            },
        };
        let sql = default_scope_insert(&source, true)
            .unwrap()
            .to_string(PostgresQueryBuilder);
        assert!(
            sql.as_str()
                .contains("INSERT INTO \"context69\".\"source_configs\""),
            "{sql}"
        );
        assert!(sql.as_str().contains("SELECT \"id\""), "{sql}");
        assert!(
            sql.as_str()
                .contains("FROM \"context69\".\"groups\" WHERE \"full_path\" = 'public'"),
            "{sql}"
        );
        assert!(
            sql.as_str()
                .contains("ON CONFLICT (\"source_key\") DO NOTHING"),
            "{sql}"
        );
    }

    #[test]
    fn status_query_preserves_group_and_checkpoint_joins() {
        let sql = status_query().to_string(PostgresQueryBuilder);
        assert!(
            sql.as_str()
                .contains("INNER JOIN \"context69\".\"groups\" AS \"g\""),
            "{sql}"
        );
        assert!(
            sql.as_str()
                .contains("LEFT JOIN \"context69\".\"source_checkpoints\" AS \"cp\""),
            "{sql}"
        );
        assert!(
            sql.as_str().contains("\"g\".\"id\" = \"sc\".\"group_id\""),
            "{sql}"
        );
        assert!(
            sql.as_str()
                .contains("\"cp\".\"source_key\" = \"sc\".\"source_key\""),
            "{sql}"
        );
    }

    #[test]
    fn invalid_example_queries_remains_sqlx_column_decode() {
        let error = example_queries(serde_json::json!([42])).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::ColumnDecode { .. })
        ));
    }

    #[test]
    fn shared_driver_error_retains_native_variant() {
        let shared = std::sync::Arc::new(sqlx::Error::RowNotFound);
        let error = sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(shared.clone()));
        let mapped = source_database_error(error);
        assert!(matches!(
            mapped.downcast_ref::<sea_orm::DbErr>(),
            Some(sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(_)))
        ));
    }

    #[test]
    fn driver_error_preserves_sqlx_downcast() {
        let error = sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(std::sync::Arc::new(
            sqlx::Error::RowNotFound,
        )));
        assert!(
            source_database_error(error)
                .downcast_ref::<sqlx::Error>()
                .is_some()
        );
    }
}
