use anyhow::{Result, anyhow};

use super::{
    PostgresSqlConnectorConfig, SourceConfig, SourceConfigRow, SourceStatus, SourceStatusRow,
    parse_sync_strategy,
};
use crate::contracts::{
    SourceConnectorType, SourceOriginStatusKind, SourceSyncStrategy, Visibility,
};

pub(super) fn row_to_source_config(row: SourceConfigRow) -> Result<SourceConfig> {
    if row.connector_type != "postgres_sql" {
        return Err(anyhow!(
            "unsupported connector_type: {}",
            row.connector_type
        ));
    }

    Ok(SourceConfig {
        key: row.source_key,
        display_name: row.display_name,
        description: row.description,
        example_queries: row.example_queries.0,
        connection: row.connection,
        sync_strategy: parse_sync_strategy(&row.sync_strategy)?,
        connector: PostgresSqlConnectorConfig {
            base_query: row.base_query,
            batch_size: row.batch_size,
        },
    })
}

pub(super) fn row_to_source_status(row: SourceStatusRow) -> SourceStatus {
    SourceStatus {
        group_key: row.group_key,
        group_path: row.group_path,
        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
        display_name: row
            .display_name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| row.source_key.clone()),
        description: row.description,
        example_queries: row.example_queries.0,
        source_key: row.source_key,
        connection: row.connection,
        has_database_url: false,
        origin_status: SourceOriginStatusKind::Unknown,
        origin_message: None,
        sync_strategy: match row.sync_strategy.as_str() {
            "cursor" => SourceSyncStrategy::Cursor,
            "full_scan" => SourceSyncStrategy::FullScan,
            _ => SourceSyncStrategy::Cursor,
        },
        connector_type: SourceConnectorType::PostgresSql,
        base_query: row.base_query,
        batch_size: row.batch_size,
        last_cursor_updated_at: row.last_cursor_updated_at,
        last_cursor_external_id: row.last_cursor_external_id,
        last_success_at: row.last_success_at,
    }
}
