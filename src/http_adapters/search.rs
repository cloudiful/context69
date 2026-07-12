use async_trait::async_trait;
use context69_search_http::SearchApi;

use crate::db::Database;
use crate::services::{auth::AuthService, query::QueryService};

#[derive(Clone)]
pub struct SearchApiAdapter {
    query: QueryService,
    auth: AuthService,
    db: Database,
}

impl SearchApiAdapter {
    pub fn new(query: QueryService, auth: AuthService, db: Database) -> Self {
        Self { query, auth, db }
    }
}

#[async_trait]
impl SearchApi for SearchApiAdapter {
    async fn search(
        &self,
        user_id: Option<i64>,
        request: crate::contracts::SearchRequest,
    ) -> anyhow::Result<crate::contracts::SearchResponse> {
        if !request.metadata_filters.is_empty() {
            let source_key = request
                .source_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("source_key is required for metadata filters"))?;
            let group_path = request
                .group_path
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("group_path is required for metadata filters"))?;
            let user_id = user_id
                .ok_or_else(|| anyhow::anyhow!("authentication required for metadata filters"))?;
            let group = self
                .db
                .get_group_for_user(user_id, group_path)
                .await?
                .ok_or_else(|| anyhow::anyhow!("group not found"))?;
            let definitions = self.db.list_metadata_indexes(group.id, source_key).await?;
            for filter in &request.metadata_filters {
                let definition = definitions
                    .iter()
                    .find(|item| item.field_path == filter.path)
                    .ok_or_else(|| {
                        anyhow::anyhow!("metadata field '{}' is not declared", filter.path)
                    })?;
                if definition.status != "ready" {
                    return Err(anyhow::anyhow!(
                        "metadata field '{}' is not ready",
                        filter.path
                    ));
                }
            }
        }
        self.query.search(user_id, request).await
    }

    async fn get_document(
        &self,
        user_id: Option<i64>,
        document_id: i64,
    ) -> anyhow::Result<crate::contracts::DocumentResponse> {
        let scope = self.auth.access_scope(user_id, None).await?;
        self.query.get_document(document_id, &scope).await
    }
}
