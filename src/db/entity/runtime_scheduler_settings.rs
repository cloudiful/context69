use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "runtime_scheduler_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    pub interval_secs: i64,
    pub run_on_start: bool,
    pub max_concurrency: i64,
    #[sea_orm(column_type = "Text")]
    pub job_id: String,
    #[sea_orm(column_type = "Text")]
    pub valkey_url: Option<String>,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(
            ColumnDef::new(Column::Singleton)
                .boolean()
                .not_null()
                .default(true)
                .primary_key(),
        )
        .col(
            ColumnDef::new(Column::IntervalSecs)
                .big_integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(Column::RunOnStart)
                .boolean()
                .not_null()
                .default(true),
        )
        .col(
            ColumnDef::new(Column::MaxConcurrency)
                .big_integer()
                .not_null(),
        )
        .col(ColumnDef::new(Column::JobId).text().not_null())
        .col(ColumnDef::new(Column::ValkeyUrl).text())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "runtime_scheduler_settings_singleton_check",
            Expr::cust("singleton"),
        ))
        .check((
            "runtime_scheduler_settings_interval_secs_check",
            Expr::cust("interval_secs > 0"),
        ))
        .check((
            "runtime_scheduler_settings_max_concurrency_check",
            Expr::cust("max_concurrency > 0"),
        ))
        .check((
            "runtime_scheduler_settings_job_id_check",
            Expr::cust("btrim(job_id) <> ''"),
        ))
        .to_owned()
}
