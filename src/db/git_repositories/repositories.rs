//! Group-scoped repository source persistence (issue #681 work unit 3B1).
//!
//! Identity is per group: two groups may register the same canonical URL and
//! ref, and each group sees only its own sources. Reads, the checkpoint update,
//! and the upsert all filter on the owning group.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::rows::GitRepositorySourceRow;
use super::types::{
    GitCheckpointUpdate, GitRepositorySettingsUpdate, NewGitRepositorySource,
    StoredGitRepositorySource,
};
use crate::db::Database;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, JoinType, Query, SelectStatement};

fn repository_projection(source_table: sea_orm::sea_query::TableRef) -> SelectStatement {
    let source = Alias::new("s");
    let group = Alias::new("g");
    let active = Alias::new("ag");
    let mut query = Query::select();
    for column in [
        "group_id",
        "repository_key",
        "connection_key",
        "provider_kind",
        "canonical_url",
        "repository_owner",
        "repository_name",
        "default_branch",
        "target_ref",
        "target_commit_sha",
        "pinned_commit_sha",
        "indexed_commit_sha",
        "index_profile",
        "include_patterns",
        "exclude_patterns",
        "interval_seconds",
        "webhook_enabled",
        "enabled",
        "retention_count",
        "created_by_user_id",
        "config_revision",
        "refresh_dirty",
        "refresh_requested_at",
        "last_refresh_attempt_at",
        "force_rebuild_requested_at",
        "deletion_requested_at",
        "last_skip_summary",
        "indexed_pipeline_version",
        "indexed_config_fingerprint",
        "index_status",
        "last_error_code",
        "last_indexed_at",
        "checkpoint_updated_at",
        "created_at",
        "updated_at",
    ] {
        query.column((source.clone(), Alias::new(column)));
    }
    query
        .column((group.clone(), Alias::new("group_key")))
        .expr_as(
            Expr::col((group.clone(), Alias::new("full_path"))),
            Alias::new("group_path"),
        )
        .expr_as(
            Expr::col((group.clone(), Alias::new("visibility"))),
            Alias::new("group_visibility"),
        )
        .expr_as(
            Expr::col((active.clone(), Alias::new("generation_key"))),
            Alias::new("active_generation_key"),
        )
        .from_as(source_table, source.clone())
        .join_as(
            JoinType::InnerJoin,
            (Alias::new("context69"), Alias::new("groups")),
            group.clone(),
            Expr::col((group, Alias::new("id"))).equals((source.clone(), Alias::new("group_id"))),
        )
        .join_as(
            JoinType::LeftJoin,
            (
                Alias::new("context69"),
                Alias::new("git_repository_active_generations"),
            ),
            active.clone(),
            Expr::col((active, Alias::new("repository_key")))
                .equals((source, Alias::new("repository_key"))),
        )
        .to_owned()
}

pub(super) fn repository_lookup_query(
    group_id: i64,
    repository_key: Option<Uuid>,
) -> SelectStatement {
    use sea_orm::sea_query::{IntoTableRef, Order};
    let mut query = repository_projection(
        (
            Alias::new("context69"),
            Alias::new("git_repository_sources"),
        )
            .into_table_ref(),
    );
    query.and_where(Expr::col((Alias::new("s"), Alias::new("group_id"))).eq(group_id));
    if let Some(key) = repository_key {
        query.and_where(Expr::col((Alias::new("s"), Alias::new("repository_key"))).eq(key));
    } else {
        query
            .order_by((Alias::new("s"), Alias::new("created_at")), Order::Desc)
            .order_by((Alias::new("s"), Alias::new("repository_key")), Order::Asc);
    }
    query
}

fn no_active_index_task() -> Expr {
    let t = Alias::new("t");
    let i = Alias::new("i");
    Expr::exists(
        Query::select()
            .expr(Expr::value(1))
            .from_as((Alias::new("context69"), Alias::new("tasks")), t.clone())
            .join_as(
                JoinType::InnerJoin,
                (Alias::new("context69"), Alias::new("task_items")),
                i.clone(),
                Expr::col((i.clone(), Alias::new("task_id"))).equals((t.clone(), Alias::new("id"))),
            )
            .and_where(Expr::col((t.clone(), Alias::new("kind"))).eq("git_index"))
            .and_where(Expr::col((t, Alias::new("status"))).is_in(["queued", "running", "waiting"]))
            .and_where(
                Expr::cust("i.payload ->> 'repository_key'")
                    .eq(Expr::cust("s.repository_key::text")),
            )
            .to_owned(),
    )
    .not()
}

fn repository_row(row: sea_orm::QueryResult) -> Result<GitRepositorySourceRow> {
    Ok(GitRepositorySourceRow {
        group_id: row.try_get("", "group_id")?,
        group_key: row.try_get("", "group_key")?,
        group_path: row.try_get("", "group_path")?,
        group_visibility: row.try_get("", "group_visibility")?,
        repository_key: row.try_get("", "repository_key")?,
        connection_key: row.try_get("", "connection_key")?,
        provider_kind: row.try_get("", "provider_kind")?,
        canonical_url: row.try_get("", "canonical_url")?,
        repository_owner: row.try_get("", "repository_owner")?,
        repository_name: row.try_get("", "repository_name")?,
        default_branch: row.try_get("", "default_branch")?,
        target_ref: row.try_get("", "target_ref")?,
        target_commit_sha: row.try_get("", "target_commit_sha")?,
        pinned_commit_sha: row.try_get("", "pinned_commit_sha")?,
        indexed_commit_sha: row.try_get("", "indexed_commit_sha")?,
        index_profile: row.try_get("", "index_profile")?,
        include_patterns: row.try_get("", "include_patterns")?,
        exclude_patterns: row.try_get("", "exclude_patterns")?,
        interval_seconds: row.try_get("", "interval_seconds")?,
        webhook_enabled: row.try_get("", "webhook_enabled")?,
        enabled: row.try_get("", "enabled")?,
        retention_count: row.try_get("", "retention_count")?,
        created_by_user_id: row.try_get("", "created_by_user_id")?,
        config_revision: row.try_get("", "config_revision")?,
        refresh_dirty: row.try_get("", "refresh_dirty")?,
        refresh_requested_at: row.try_get("", "refresh_requested_at")?,
        last_refresh_attempt_at: row.try_get("", "last_refresh_attempt_at")?,
        force_rebuild_requested_at: row.try_get("", "force_rebuild_requested_at")?,
        deletion_requested_at: row.try_get("", "deletion_requested_at")?,
        last_skip_summary: row.try_get("", "last_skip_summary")?,
        indexed_pipeline_version: row.try_get("", "indexed_pipeline_version")?,
        indexed_config_fingerprint: row.try_get("", "indexed_config_fingerprint")?,
        index_status: row.try_get("", "index_status")?,
        last_error_code: row.try_get("", "last_error_code")?,
        last_indexed_at: row.try_get("", "last_indexed_at")?,
        checkpoint_updated_at: row.try_get("", "checkpoint_updated_at")?,
        active_generation_key: row.try_get("", "active_generation_key")?,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}

pub(super) fn checkpoint_query(
    group_id: i64,
    repository_key: Uuid,
    checkpoint: &GitCheckpointUpdate,
) -> sea_orm::sea_query::WithQuery {
    use sea_orm::sea_query::{CommonTableExpression, Func, IntoTableRef};
    let mut update = Query::update();
    update.table((
        Alias::new("context69"),
        Alias::new("git_repository_sources"),
    ));
    update.value(
        Alias::new("target_commit_sha"),
        Func::coalesce([
            Expr::value(checkpoint.target_commit_sha.as_deref()),
            Expr::col(Alias::new("target_commit_sha")),
        ]),
    );
    update.value(
        Alias::new("indexed_commit_sha"),
        Func::coalesce([
            Expr::value(checkpoint.indexed_commit_sha.as_deref()),
            Expr::col(Alias::new("indexed_commit_sha")),
        ]),
    );
    update.value(Alias::new("index_status"), checkpoint.index_status.as_str());
    update.value(
        Alias::new("indexed_pipeline_version"),
        Func::coalesce([
            Expr::value(checkpoint.pipeline_version.map(i64::from)),
            Expr::col(Alias::new("indexed_pipeline_version")),
        ]),
    );
    update.value(
        Alias::new("indexed_config_fingerprint"),
        Func::coalesce([
            Expr::value(checkpoint.config_fingerprint.as_deref()),
            Expr::col(Alias::new("indexed_config_fingerprint")),
        ]),
    );
    update.value(
        Alias::new("last_skip_summary"),
        Func::coalesce([
            Expr::value(
                checkpoint
                    .skip_summary
                    .map(super::types::skip_summary_value),
            ),
            Expr::col(Alias::new("last_skip_summary")),
        ]),
    );
    if checkpoint.index_status.as_str() == "ready" {
        update.value(Alias::new("last_error_code"), Option::<String>::None);
    }
    if checkpoint.indexed_commit_sha.is_some() {
        update.value(Alias::new("last_indexed_at"), Expr::current_timestamp());
    }
    update
        .value(
            Alias::new("checkpoint_updated_at"),
            Expr::current_timestamp(),
        )
        .value(Alias::new("updated_at"), Expr::current_timestamp())
        .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
        .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
        .returning_all();
    let mut cte = CommonTableExpression::new();
    cte.table_name(Alias::new("updated"))
        .query(update.to_owned());
    repository_projection(Alias::new("updated").into_table_ref())
        .with(Query::with().cte(cte).to_owned())
}

impl Database {
    /// Registers or updates a repository source owned by `group_id`.
    ///
    /// The conflict target is `(group_id, canonical_url, target_ref)`, so an
    /// upsert updates the calling group's own registration and cannot create or
    /// take over another group's registration of the same repository ref.
    pub async fn upsert_git_repository_source(
        &self,
        group_id: i64,
        source: &NewGitRepositorySource,
    ) -> Result<StoredGitRepositorySource> {
        // The shared ancestor-key lock and the insert share one transaction, so
        // a concurrent delete of this group or any ancestor cannot cascade the
        // group between the ownership check and this registration. See
        // `lock_git_group_source_registration.sql`.
        let mut tx = self.pool.begin().await?;
        sqlx::query_file!(
            "src/sql/db/git_repositories/lock_git_group_source_registration.sql",
            group_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file_scalar!(
            "src/sql/db/git_repositories/lock_git_group_source_ancestors.sql",
            group_id
        )
        .fetch_all(&mut *tx)
        .await?;
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/upsert_git_repository_source.sql",
            group_id,
            source.connection_key,
            source.provider.as_str(),
            source.canonical_url,
            source.owner,
            source.name,
            source.default_branch,
            source.target_ref,
            source.pinned_commit_sha,
            source.index_profile.as_str(),
            &source.include_patterns,
            &source.exclude_patterns,
            source.interval_seconds,
            source.webhook_enabled,
            source.enabled,
            source.retention_count,
            source.created_by_user_id
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        StoredGitRepositorySource::from_row(row)
    }

    /// Reads one source owned by `group_id`; a source of another group reads as
    /// absent.
    pub async fn get_git_repository_source(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitRepositorySource>> {
        use sea_orm::ConnectionTrait;
        let query = repository_lookup_query(group_id, Some(repository_key));
        self.orm
            .query_one(&query)
            .await?
            .map(|row| StoredGitRepositorySource::from_row(repository_row(row)?))
            .transpose()
    }

    /// Lists the sources of one group only.
    pub async fn list_git_repository_sources(
        &self,
        group_id: i64,
    ) -> Result<Vec<StoredGitRepositorySource>> {
        use sea_orm::ConnectionTrait;
        let query = repository_lookup_query(group_id, None);
        self.orm
            .query_all(&query)
            .await?
            .into_iter()
            .map(|row| StoredGitRepositorySource::from_row(repository_row(row)?))
            .collect()
    }

    /// Advances the incremental-sync checkpoint of a source owned by
    /// `group_id`, reporting `None` when the source belongs to another group.
    pub async fn update_git_repository_checkpoint(
        &self,
        group_id: i64,
        repository_key: Uuid,
        checkpoint: &GitCheckpointUpdate,
    ) -> Result<Option<StoredGitRepositorySource>> {
        use sea_orm::ConnectionTrait;
        let query = checkpoint_query(group_id, repository_key, checkpoint);
        self.orm
            .query_one(&query)
            .await?
            .map(|row| StoredGitRepositorySource::from_row(repository_row(row)?))
            .transpose()
    }

    /// Moves a source to `indexing` and records the resolved target, but only
    /// while the configuration revision and observed target a run read are still
    /// current.
    pub async fn begin_git_repository_index(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
        expected_target_commit_sha: Option<&str>,
        observed_commit_sha: &str,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("target_commit_sha"), observed_commit_sha)
            .value(Alias::new("index_status"), "indexing")
            .value(
                Alias::new("checkpoint_updated_at"),
                Expr::current_timestamp(),
            )
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("config_revision")).eq(expected_config_revision))
            .and_where(Expr::cust_with_values(
                "target_commit_sha IS NOT DISTINCT FROM $1",
                [expected_target_commit_sha],
            ))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Refreshes the checkpoint of a same-target no-op only while its captured
    /// configuration revision and observed target are still current.
    pub async fn complete_git_repository_noop(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
        expected_target_commit_sha: Option<&str>,
        observed_commit_sha: &str,
        pipeline_version: u32,
        config_fingerprint: &str,
        skip_summary: &crate::contracts::sources::GitRepositorySkipSummary,
    ) -> Result<bool> {
        let skip_summary = super::types::skip_summary_value(*skip_summary);
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("target_commit_sha"), observed_commit_sha)
            .value(Alias::new("indexed_commit_sha"), observed_commit_sha)
            .value(Alias::new("index_status"), "ready")
            .value(
                Alias::new("indexed_pipeline_version"),
                i64::from(pipeline_version),
            )
            .value(Alias::new("indexed_config_fingerprint"), config_fingerprint)
            .value(Alias::new("last_skip_summary"), skip_summary)
            .value(Alias::new("last_error_code"), Option::<String>::None)
            .value(Alias::new("last_indexed_at"), Expr::current_timestamp())
            .value(
                Alias::new("checkpoint_updated_at"),
                Expr::current_timestamp(),
            )
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("config_revision")).eq(expected_config_revision))
            .and_where(Expr::cust_with_values(
                "target_commit_sha IS NOT DISTINCT FROM $1",
                [expected_target_commit_sha],
            ))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Marks the source failed only while the attempt's configuration revision
    /// is still current.
    pub async fn fail_git_repository_index(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("index_status"), "failed")
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("config_revision")).eq(expected_config_revision))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Records one durable refresh demand on an enabled source of `group_id`.
    pub async fn request_git_refresh(&self, group_id: i64, repository_key: Uuid) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("refresh_dirty"), true)
            .value(
                Alias::new("refresh_requested_at"),
                Expr::current_timestamp(),
            )
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Records a refresh demand inside the caller's submission transaction, so
    /// admitting a reused task and marking the demand commit together.
    pub(crate) async fn request_git_refresh_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/request_git_refresh.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.is_some())
    }

    /// Records a force-rebuild demand on an enabled source.
    pub async fn request_git_force_rebuild(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(
                Alias::new("force_rebuild_requested_at"),
                Expr::current_timestamp(),
            )
            .value(Alias::new("refresh_dirty"), true)
            .value(
                Alias::new("refresh_requested_at"),
                Expr::current_timestamp(),
            )
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Records a force-rebuild demand inside the caller's submission
    /// transaction, alongside the refresh marker that schedules the follow-up.
    pub(crate) async fn request_git_force_rebuild_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/request_git_force_rebuild.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.is_some())
    }

    /// Bounds retries by recording that a refresh attempt was admitted.
    pub async fn mark_git_refresh_attempt(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(
                Alias::new("last_refresh_attempt_at"),
                Expr::current_timestamp(),
            )
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Clears a demand only when no newer request arrived after `observed_at`.
    pub async fn consume_git_refresh_request(
        &self,
        group_id: i64,
        repository_key: Uuid,
        observed_at: DateTime<Utc>,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("refresh_dirty"), false)
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("refresh_dirty")).eq(true))
            .and_where(Expr::col(Alias::new("refresh_requested_at")).is_not_null())
            .and_where(Expr::col(Alias::new("refresh_requested_at")).lte(observed_at))
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Due refresh sources, bounded by `limit`.
    pub async fn list_due_git_refresh_sources(&self, limit: i64) -> Result<Vec<(i64, Uuid)>> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{NullOrdering, Order},
        };
        if limit < 0 {
            let rows = sqlx::query_file!(
                "src/sql/db/git_repositories/list_due_git_refresh_sources.sql",
                limit
            )
            .fetch_all(&self.pool)
            .await?;
            return Ok(rows
                .into_iter()
                .map(|row| (row.group_id, row.repository_key))
                .collect());
        }
        let s = Alias::new("s");
        let attempt_due = Expr::col((s.clone(), Alias::new("last_refresh_attempt_at")))
            .is_null()
            .or(
                Expr::col((s.clone(), Alias::new("last_refresh_attempt_at")))
                    .lte(Expr::cust("now() - interval '60 seconds'")),
            );
        let dirty = Expr::col((s.clone(), Alias::new("refresh_dirty")))
            .eq(true)
            .and(attempt_due.clone());
        let interval_due = Expr::col((s.clone(), Alias::new("interval_seconds"))).is_not_null()
            .and(Expr::cust("GREATEST(COALESCE(s.last_indexed_at, s.created_at), COALESCE(s.last_refresh_attempt_at, s.created_at)) + make_interval(secs => s.interval_seconds)").lte(Expr::current_timestamp()))
            .and(attempt_due);
        let query = Query::select()
            .columns([
                (s.clone(), Alias::new("group_id")),
                (s.clone(), Alias::new("repository_key")),
            ])
            .from_as(
                (
                    Alias::new("context69"),
                    Alias::new("git_repository_sources"),
                ),
                s.clone(),
            )
            .and_where(Expr::col((s.clone(), Alias::new("enabled"))).eq(true))
            .and_where(Expr::col((s.clone(), Alias::new("deletion_requested_at"))).is_null())
            .and_where(Expr::col((s.clone(), Alias::new("index_status"))).ne("disabled"))
            .and_where(no_active_index_task())
            .and_where(dirty.or(interval_due))
            .order_by_with_nulls(
                (s.clone(), Alias::new("last_refresh_attempt_at")),
                Order::Asc,
                NullOrdering::First,
            )
            .order_by((s, Alias::new("updated_at")), Order::Asc)
            .limit(limit as u64)
            .to_owned();
        self.orm
            .query_all(&query)
            .await?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get("", "group_id")?,
                    row.try_get("", "repository_key")?,
                ))
            })
            .collect()
    }

    /// Sources with a pending delete whose cleanup can be retried.
    pub async fn list_pending_git_deletions(&self, limit: i64) -> Result<Vec<(i64, Uuid)>> {
        use sea_orm::{ConnectionTrait, sea_query::Order};
        if limit < 0 {
            let rows = sqlx::query_file!(
                "src/sql/db/git_repositories/list_pending_git_deletions.sql",
                limit
            )
            .fetch_all(&self.pool)
            .await?;
            return Ok(rows
                .into_iter()
                .map(|row| (row.group_id, row.repository_key))
                .collect());
        }
        let s = Alias::new("s");
        let retry = Expr::col((s.clone(), Alias::new("deletion_next_retry_at")));
        let query = Query::select()
            .columns([
                (s.clone(), Alias::new("group_id")),
                (s.clone(), Alias::new("repository_key")),
            ])
            .from_as(
                (
                    Alias::new("context69"),
                    Alias::new("git_repository_sources"),
                ),
                s.clone(),
            )
            .and_where(Expr::col((s.clone(), Alias::new("deletion_requested_at"))).is_not_null())
            .and_where(
                retry
                    .clone()
                    .is_null()
                    .or(retry.lte(Expr::current_timestamp())),
            )
            .and_where(no_active_index_task())
            .order_by_expr(
                Expr::cust("COALESCE(s.deletion_next_retry_at, to_timestamp(0))"),
                Order::Asc,
            )
            .order_by((s.clone(), Alias::new("deletion_attempts")), Order::Asc)
            .order_by((s.clone(), Alias::new("deletion_requested_at")), Order::Asc)
            .order_by((s, Alias::new("repository_key")), Order::Asc)
            .limit(limit as u64)
            .to_owned();
        self.orm
            .query_all(&query)
            .await?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get("", "group_id")?,
                    row.try_get("", "repository_key")?,
                ))
            })
            .collect()
    }

    /// Marks a source for deletion and stops further admissions.
    pub async fn mark_git_repository_deletion_requested(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Func, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("enabled"), false)
            .value(Alias::new("index_status"), "disabled")
            .value(
                Alias::new("deletion_requested_at"),
                Func::coalesce([
                    Expr::col(Alias::new("deletion_requested_at")),
                    Expr::current_timestamp(),
                ]),
            )
            .value(Alias::new("refresh_dirty"), false)
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Marks a source for deletion inside the caller's admission transaction.
    pub(crate) async fn mark_git_repository_deletion_requested_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_repository_deletion_requested.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.is_some())
    }

    /// Records a failed source-deletion cleanup and its bounded retry time.
    pub(crate) async fn mark_git_repository_deletion_retry(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update().table((Alias::new("context69"), Alias::new("git_repository_sources")))
            .value(Alias::new("deletion_attempts"), Expr::col(Alias::new("deletion_attempts")).add(1))
            .value(Alias::new("deletion_attempted_at"), Expr::current_timestamp())
            .value(Alias::new("deletion_next_retry_at"), Expr::cust("now() + LEAST(interval '1 hour', interval '30 seconds' * power(2, LEAST(deletion_attempts, 7)))"))
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_not_null()).to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    /// Removes a source after its vector content is gone.
    pub async fn delete_git_repository_source(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        use crate::db::entity::git_repository_sources::{Column, Entity};
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let result = Entity::delete_many()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::RepositoryKey.eq(repository_key))
            .exec(&self.orm)
            .await?;
        Ok(result.rows_affected > 0)
    }

    /// Resolves the user a background refresh is attributed to.
    pub async fn resolve_git_refresh_actor(
        &self,
        group_id: i64,
        preferred_user_id: Option<i64>,
    ) -> Result<Option<i64>> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/resolve_git_refresh_actor.sql",
            group_id,
            preferred_user_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|row| row.user_id))
    }

    /// Replaces the editable target, profile, and settings of one source owned
    /// by `group_id`.
    pub async fn update_git_repository_source_settings(
        &self,
        group_id: i64,
        repository_key: Uuid,
        update: &GitRepositorySettingsUpdate<'_>,
    ) -> Result<Option<StoredGitRepositorySource>> {
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/update_git_repository_source_settings.sql",
            group_id,
            repository_key,
            update.target_ref,
            update.pinned_commit_action,
            update.pinned_commit_value,
            update.index_profile,
            update.include_patterns,
            update.exclude_patterns,
            update.interval_seconds,
            update.webhook_enabled,
            update.enabled,
            update.retention_count
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitRepositorySource::from_row).transpose()
    }

    /// Records the bounded stable code of a terminal index failure only while
    /// the attempt's configuration revision is still current.
    pub async fn mark_git_repository_failure(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
        error_code: &str,
    ) -> Result<bool> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query},
        };
        let query = Query::update()
            .table((
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            ))
            .value(Alias::new("index_status"), "failed")
            .value(Alias::new("last_error_code"), error_code)
            .value(Alias::new("updated_at"), Expr::current_timestamp())
            .and_where(Expr::col(Alias::new("group_id")).eq(group_id))
            .and_where(Expr::col(Alias::new("repository_key")).eq(repository_key))
            .and_where(Expr::col(Alias::new("config_revision")).eq(expected_config_revision))
            .and_where(Expr::col(Alias::new("enabled")).eq(true))
            .and_where(Expr::col(Alias::new("deletion_requested_at")).is_null())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }
}

/// One held per-repository index lifetime advisory lock.
///
/// The lock lives in a transaction on its own pooled connection, so a long
/// provider or vector pass never keeps table row locks or a snapshot open. The
/// transaction is only used for the advisory lock; every data statement takes
/// its own connection. Dropping the guard queues the rollback that releases the
/// lock, so cancellation and errors release it too.
pub(crate) struct GitIndexLifetimeGuard {
    tx: Option<Transaction<'static, Postgres>>,
}

impl GitIndexLifetimeGuard {
    /// Waits for the lifetime lock, as an index run must.
    pub(crate) async fn acquire(pool: &PgPool, repository_key: Uuid) -> Result<Self> {
        let mut tx = pool.begin().await?;
        let key = git_index_lifetime_lock_key(repository_key);
        sqlx::query_file!(
            "src/sql/db/git_repositories/lock_git_index_lifetime.sql",
            key
        )
        .execute(&mut *tx)
        .await?;
        Ok(Self { tx: Some(tx) })
    }

    /// Takes the lifetime lock only if it is free, as cleanup and source
    /// deletion must so they can defer instead of blocking an active run.
    pub(crate) async fn try_acquire(pool: &PgPool, repository_key: Uuid) -> Result<Option<Self>> {
        let mut tx = pool.begin().await?;
        let key = git_index_lifetime_lock_key(repository_key);
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/try_lock_git_index_lifetime.sql",
            key
        )
        .fetch_one(&mut *tx)
        .await?;
        if row.acquired != Some(true) {
            tx.rollback().await?;
            return Ok(None);
        }
        Ok(Some(Self { tx: Some(tx) }))
    }

    /// Releases the lock explicitly before the caller reports its outcome.
    pub(crate) async fn release(mut self) -> Result<()> {
        if let Some(tx) = self.tx.take() {
            tx.commit().await?;
        }
        Ok(())
    }
}

/// The lifetime lock key is separate from the short admission lock so holding
/// one never blocks admitting or observing the other.
fn git_index_lifetime_lock_key(repository_key: Uuid) -> String {
    format!("context69.git_index_lifetime:{repository_key}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::{IntoTableRef, PostgresQueryBuilder};

    #[test]
    fn repository_projection_joins_authoritative_ownership_and_generation() {
        let sql = repository_projection(
            (
                Alias::new("context69"),
                Alias::new("git_repository_sources"),
            )
                .into_table_ref(),
        )
        .to_string(PostgresQueryBuilder);
        assert!(sql.contains("\"context69\".\"groups\" AS \"g\""), "{sql}");
        assert!(
            sql.contains("LEFT JOIN \"context69\".\"git_repository_active_generations\" AS \"ag\""),
            "{sql}"
        );
        assert!(sql.contains("\"g\".\"id\" = \"s\".\"group_id\""), "{sql}");
    }

    #[test]
    fn scheduler_excludes_active_index_tasks_for_the_same_repository() {
        let sql = Query::select()
            .expr(no_active_index_task())
            .to_string(PostgresQueryBuilder);
        assert!(sql.contains("NOT EXISTS"), "{sql}");
        assert!(
            sql.contains("(i.payload ->> 'repository_key') = (s.repository_key::text)"),
            "{sql}"
        );
        assert!(sql.contains("'queued', 'running', 'waiting'"), "{sql}");
    }
}
