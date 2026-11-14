use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "library_dependency_gates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub dependency_key: String,
    pub state: String,
    pub failure_count: i32,
    pub next_probe_at: Option<DateTimeUtc>,
    pub last_error: Option<String>,
    pub configuration_fingerprint: Option<String>,
    pub probe_lease_token: Option<Uuid>,
    pub probe_lease_expires_at: Option<DateTimeUtc>,
    pub last_transition_at: DateTimeUtc,
    pub last_success_at: Option<DateTimeUtc>,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the dependency gate table.
///
/// The table carries no foreign keys: it describes the state of an external
/// dependency. The declaration describes the columns, the primary key and the
/// checks of the applied schema and is never executed.
pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(
            ColumnDef::new(Column::DependencyKey)
                .text()
                .not_null()
                .primary_key(),
        )
        .col(
            ColumnDef::new(Column::State)
                .text()
                .not_null()
                .default("closed"),
        )
        .col(
            ColumnDef::new(Column::FailureCount)
                .integer()
                .not_null()
                .default(0),
        )
        .col(ColumnDef::new(Column::NextProbeAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::LastError).text())
        .col(ColumnDef::new(Column::ConfigurationFingerprint).text())
        .col(ColumnDef::new(Column::ProbeLeaseToken).uuid())
        .col(ColumnDef::new(Column::ProbeLeaseExpiresAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::LastTransitionAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(ColumnDef::new(Column::LastSuccessAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "chk_library_dependency_gates_state",
            Expr::cust("state IN ('closed', 'open', 'half_open')"),
        ))
        .check((
            "chk_library_dependency_gates_failure_count",
            Expr::cust("failure_count >= 0"),
        ))
        .to_owned()
}

/// The probe scan of the gates that are due.
pub fn probe_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_dependency_gates_probe")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::State)
        .col(Column::NextProbeAt)
        .to_owned()
}
