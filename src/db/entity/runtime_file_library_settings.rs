use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(
    schema_name = "context69",
    table_name = "runtime_file_library_settings"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    #[sea_orm(column_type = "Text")]
    pub storage_root: String,
    pub max_upload_size_mb: i64,
    pub trusted_proxy_enabled: bool,
    #[sea_orm(column_type = "Text")]
    pub s3_endpoint: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub s3_region: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub s3_bucket: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub s3_prefix: Option<String>,
    pub s3_path_style: bool,
    #[sea_orm(column_type = "Text")]
    pub s3_access_key: Option<String>,
    pub registry_max_ids_per_file: i64,
    pub registry_max_ids_per_run: i64,
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
        .col(ColumnDef::new(Column::StorageRoot).text().not_null())
        .col(
            ColumnDef::new(Column::MaxUploadSizeMb)
                .big_integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(Column::TrustedProxyEnabled)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::S3Endpoint).text())
        .col(ColumnDef::new(Column::S3Region).text())
        .col(ColumnDef::new(Column::S3Bucket).text())
        .col(ColumnDef::new(Column::S3Prefix).text())
        .col(
            ColumnDef::new(Column::S3PathStyle)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::S3AccessKey).text())
        .col(
            ColumnDef::new(Column::RegistryMaxIdsPerFile)
                .big_integer()
                .not_null()
                .default(16384i64),
        )
        .col(
            ColumnDef::new(Column::RegistryMaxIdsPerRun)
                .big_integer()
                .not_null()
                .default(8192i64),
        )
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "runtime_file_library_settings_singleton_check",
            Expr::cust("singleton"),
        ))
        .check((
            "runtime_file_library_settings_storage_root_check",
            Expr::cust("btrim(storage_root) <> ''"),
        ))
        .check((
            "runtime_file_library_settings_max_upload_size_mb_check",
            Expr::cust("max_upload_size_mb > 0"),
        ))
        .check((
            "runtime_file_library_registry_caps_positive",
            Expr::cust("registry_max_ids_per_file > 0 AND registry_max_ids_per_run > 0"),
        ))
        .check((
            "runtime_file_library_registry_caps_ordered",
            Expr::cust("registry_max_ids_per_run <= registry_max_ids_per_file"),
        ))
        .to_owned()
}
