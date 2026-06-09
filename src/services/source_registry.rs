use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::{
    config::SourceConfig,
    sources::{SourceConnector, postgres_sql::PostgresSqlSourceConnector},
};

pub struct SourceRegistry {
    configs: HashMap<String, SourceConfig>,
    connectors: HashMap<String, Arc<dyn SourceConnector>>,
    locks: HashMap<String, Arc<Mutex<()>>>,
}

impl SourceRegistry {
    pub fn new(
        source_configs: Vec<SourceConfig>,
        pools: &HashMap<String, PgPool>,
        existing_locks: &HashMap<String, Arc<Mutex<()>>>,
    ) -> Result<Self> {
        let mut configs = HashMap::new();
        let mut connectors = HashMap::new();
        let mut locks = HashMap::new();

        for source in source_configs {
            let source_key = source.key.clone();

            locks.insert(
                source_key.clone(),
                existing_locks
                    .get(&source_key)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Mutex::new(()))),
            );

            if let Some(pool) = pools.get(&source.connection).cloned() {
                let connector = PostgresSqlSourceConnector::new(
                    pool,
                    source_key.clone(),
                    source.sync_strategy,
                    source.connector.clone(),
                );
                connectors.insert(
                    source_key.clone(),
                    Arc::new(connector) as Arc<dyn SourceConnector>,
                );
            }
            configs.insert(source_key, source);
        }

        Ok(Self {
            configs,
            connectors,
            locks,
        })
    }

    pub fn source_keys(&self) -> Vec<String> {
        let mut keys = self.configs.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub fn config(&self, source_key: &str) -> Option<SourceConfig> {
        self.configs.get(source_key).cloned()
    }

    pub fn connector(&self, source_key: &str) -> Option<Arc<dyn SourceConnector>> {
        self.connectors.get(source_key).cloned()
    }

    pub fn lock(&self, source_key: &str) -> Option<Arc<Mutex<()>>> {
        self.locks.get(source_key).cloned()
    }

    pub fn connectors(&self) -> Vec<(String, Arc<dyn SourceConnector>)> {
        let mut entries = self
            .connectors
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries
    }

    pub fn locks_snapshot(&self) -> HashMap<String, Arc<Mutex<()>>> {
        self.locks.clone()
    }
}
