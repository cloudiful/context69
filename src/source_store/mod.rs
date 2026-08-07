use chrono::{DateTime, Utc};
use sqlx::{FromRow, types::Json};

use crate::config::{PostgresSqlConnectorConfig, SourceConfig, parse_sync_strategy};
use crate::contracts::{SourceConfigInput, SourceStatus};
use crate::db::Database;

mod mappers;
mod queries;
mod validation;

use mappers::{row_to_source_config, row_to_source_status};
pub use validation::MAX_SOURCE_EXAMPLE_QUERIES;

const MAX_SOURCE_EXAMPLE_QUERY_LEN: usize = 120;

#[derive(Clone)]
pub struct SourceStore {
    db: Database,
}

#[derive(Debug, Clone)]
pub struct SourceScope {
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: crate::contracts::Visibility,
}

#[derive(Debug, Clone, FromRow)]
struct SourceStatusRow {
    group_id: i64,
    group_key: String,
    group_path: String,
    visibility: String,
    source_key: String,
    display_name: Option<String>,
    description: Option<String>,
    example_queries: Json<Vec<String>>,
    connection: String,
    sync_strategy: String,
    connector_type: String,
    base_query: String,
    batch_size: i64,
    last_cursor_updated_at: Option<DateTime<Utc>>,
    last_cursor_external_id: Option<String>,
    last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct SourceConfigRow {
    source_key: String,
    display_name: Option<String>,
    description: Option<String>,
    example_queries: Json<Vec<String>>,
    connection: String,
    sync_strategy: String,
    connector_type: String,
    base_query: String,
    batch_size: i64,
}

impl SourceStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::{SourceConfigInput, SourceConnectorType, SourceSyncStrategy};

    use super::SourceStore;

    fn connections() -> Vec<String> {
        vec!["gov-info".to_string()]
    }

    #[test]
    fn validates_source_input_against_known_connections() {
        let input = SourceConfigInput {
            source_id: None,
            source_key: "gov_documents".to_string(),
            display_name: None,
            description: None,
            example_queries: Vec::new(),
            connection: "missing".to_string(),
            database_url: None,
            sync_strategy: SourceSyncStrategy::Cursor,
            connector_type: SourceConnectorType::PostgresSql,
            base_query: "SELECT 1".to_string(),
            batch_size: 200,
            visibility: None,
        };

        let error = SourceStore::validate_source_input(&input, &connections())
            .expect_err("expected invalid connection");
        assert!(error.to_string().contains("unknown connection"));
    }

    #[test]
    fn rejects_invalid_batch_size() {
        let input = SourceConfigInput {
            source_id: None,
            source_key: "gov_documents".to_string(),
            display_name: None,
            description: None,
            example_queries: Vec::new(),
            connection: "gov-info".to_string(),
            database_url: None,
            sync_strategy: SourceSyncStrategy::Cursor,
            connector_type: SourceConnectorType::PostgresSql,
            base_query: "SELECT 1".to_string(),
            batch_size: 0,
            visibility: None,
        };

        let error = SourceStore::validate_source_input(&input, &connections())
            .expect_err("expected invalid batch size");
        assert!(error.to_string().contains("batch_size"));
    }

    #[test]
    fn normalizes_source_metadata_fields() {
        let input = SourceConfigInput {
            source_id: None,
            source_key: " gov_documents ".to_string(),
            display_name: Some(" 国务院/部委政策公文 ".to_string()),
            description: Some(" 覆盖正式政策公文 ".to_string()),
            example_queries: vec![
                " 新能源汽车 购置税 政策 ".to_string(),
                "".to_string(),
                "新能源汽车 购置税 政策".to_string(),
                "国务院 关于 数据要素 的意见".to_string(),
            ],
            connection: "gov-info".to_string(),
            database_url: None,
            sync_strategy: SourceSyncStrategy::Cursor,
            connector_type: SourceConnectorType::PostgresSql,
            base_query: "SELECT 1".to_string(),
            batch_size: 200,
            visibility: None,
        };

        let source =
            SourceStore::validate_source_input(&input, &connections()).expect("source input");
        assert_eq!(source.display_name.as_deref(), Some("国务院/部委政策公文"));
        assert_eq!(source.description.as_deref(), Some("覆盖正式政策公文"));
        assert_eq!(
            source.example_queries,
            vec![
                "新能源汽车 购置税 政策".to_string(),
                "国务院 关于 数据要素 的意见".to_string()
            ]
        );
    }

    #[test]
    fn rejects_too_many_example_queries() {
        let input = SourceConfigInput {
            source_id: None,
            source_key: "gov_documents".to_string(),
            display_name: None,
            description: None,
            example_queries: vec![
                "1".to_string(),
                "2".to_string(),
                "3".to_string(),
                "4".to_string(),
                "5".to_string(),
                "6".to_string(),
                "7".to_string(),
            ],
            connection: "gov-info".to_string(),
            database_url: None,
            sync_strategy: SourceSyncStrategy::Cursor,
            connector_type: SourceConnectorType::PostgresSql,
            base_query: "SELECT 1".to_string(),
            batch_size: 200,
            visibility: None,
        };

        let error = SourceStore::validate_source_input(&input, &connections())
            .expect_err("expected invalid example query count");
        assert!(error.to_string().contains("example_queries"));
    }
}
