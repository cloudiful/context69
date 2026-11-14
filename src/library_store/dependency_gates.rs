use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, EntityTrait, QueryOrder, QuerySelect};
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;
use crate::db::entity::library_dependency_gates;

#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
pub struct DependencyGateRecord {
    pub dependency_key: String,
    pub state: String,
    pub failure_count: i32,
    pub next_probe_at: Option<DateTime<Utc>>,
    pub probe_lease_expires_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub probe_lease_token: Option<Uuid>,
    pub last_transition_at: DateTime<Utc>,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DependencyGateTransition {
    pub dependency_key: String,
    pub state: String,
    pub transitioned: bool,
}

impl LibraryStore {
    pub async fn ensure_dependency_gate(&self, dependency_key: &str) -> Result<()> {
        let model = library_dependency_gates::ActiveModel {
            dependency_key: Set(dependency_key.to_owned()),
            state: sea_orm::ActiveValue::NotSet,
            failure_count: sea_orm::ActiveValue::NotSet,
            next_probe_at: sea_orm::ActiveValue::NotSet,
            last_error: sea_orm::ActiveValue::NotSet,
            configuration_fingerprint: sea_orm::ActiveValue::NotSet,
            probe_lease_token: sea_orm::ActiveValue::NotSet,
            probe_lease_expires_at: sea_orm::ActiveValue::NotSet,
            last_transition_at: sea_orm::ActiveValue::NotSet,
            last_success_at: sea_orm::ActiveValue::NotSet,
            updated_at: sea_orm::ActiveValue::NotSet,
        };
        library_dependency_gates::Entity::insert(model)
            .on_conflict(
                OnConflict::column(library_dependency_gates::Column::DependencyKey)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(self.db.orm())
            .await?;
        Ok(())
    }

    pub async fn list_dependency_gates(&self) -> Result<Vec<DependencyGateRecord>> {
        let rows = library_dependency_gates::Entity::find()
            .select_only()
            .column(library_dependency_gates::Column::DependencyKey)
            .column(library_dependency_gates::Column::State)
            .column(library_dependency_gates::Column::FailureCount)
            .column(library_dependency_gates::Column::NextProbeAt)
            .column(library_dependency_gates::Column::ProbeLeaseExpiresAt)
            .column(library_dependency_gates::Column::LastError)
            .column(library_dependency_gates::Column::ProbeLeaseToken)
            .column(library_dependency_gates::Column::LastTransitionAt)
            .column(library_dependency_gates::Column::LastSuccessAt)
            .order_by_asc(library_dependency_gates::Column::DependencyKey)
            .into_model::<DependencyGateRecord>()
            .all(self.db.orm())
            .await?;
        Ok(rows)
    }

    pub async fn configure_dependency_gate(
        &self,
        dependency_key: &str,
        configured: bool,
        error: Option<&str>,
        configuration_fingerprint: &str,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/configure.sql",
            dependency_key,
            configured,
            error,
            configuration_fingerprint
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn reserve_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/reserve_probe.sql",
            dependency_key,
            lease_token,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn record_dependency_success(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/success.sql",
            dependency_key,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn record_dependency_failure(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
        error: &str,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/failure.sql",
            dependency_key,
            lease_token,
            error
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn abandon_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/abandon_probe.sql",
            dependency_key,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn release_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/release_probe.sql",
            dependency_key,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?)
    }
}
