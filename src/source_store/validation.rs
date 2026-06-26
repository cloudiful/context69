use std::collections::HashSet;

use anyhow::{Result, anyhow};

use super::{
    MAX_SOURCE_EXAMPLE_QUERY_LEN, PostgresSqlConnectorConfig, SourceConfig, SourceConfigInput,
    SourceStore, parse_sync_strategy,
};

pub const MAX_SOURCE_EXAMPLE_QUERIES: usize = 6;

impl SourceStore {
    pub fn validate_source_input(
        input: &SourceConfigInput,
        connection_names: &[String],
    ) -> Result<SourceConfig> {
        let display_name = input
            .display_name
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let description = input
            .description
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let example_queries = input
            .example_queries
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .fold(Vec::new(), |mut acc, value| {
                let value = value.to_string();
                if !acc.contains(&value) {
                    acc.push(value);
                }
                acc
            });

        let source_key = input.source_key.trim();
        if source_key.is_empty() {
            return Err(anyhow!("source_key must not be empty"));
        }

        let connection = input.connection.trim();
        if connection.is_empty() {
            return Err(anyhow!("connection must not be empty"));
        }

        let known_connections = connection_names
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if !known_connections.contains(connection) {
            return Err(anyhow!("unknown connection {connection}"));
        }

        if input.connector_type != "postgres_sql" {
            return Err(anyhow!(
                "unsupported connector_type: {}",
                input.connector_type
            ));
        }

        let base_query = input.base_query.trim();
        if base_query.is_empty() {
            return Err(anyhow!("base_query must not be empty"));
        }

        if input.batch_size <= 0 {
            return Err(anyhow!("batch_size must be greater than 0"));
        }
        if example_queries.len() > MAX_SOURCE_EXAMPLE_QUERIES {
            return Err(anyhow!(
                "example_queries must contain at most {MAX_SOURCE_EXAMPLE_QUERIES} items"
            ));
        }
        if example_queries
            .iter()
            .any(|query| query.chars().count() > MAX_SOURCE_EXAMPLE_QUERY_LEN)
        {
            return Err(anyhow!(
                "example_queries items must be at most {MAX_SOURCE_EXAMPLE_QUERY_LEN} characters"
            ));
        }

        Ok(SourceConfig {
            key: source_key.to_string(),
            display_name,
            description,
            example_queries,
            connection: connection.to_string(),
            sync_strategy: parse_sync_strategy(&input.sync_strategy)?,
            connector: PostgresSqlConnectorConfig {
                base_query: base_query.to_string(),
                batch_size: input.batch_size,
            },
        })
    }
}
