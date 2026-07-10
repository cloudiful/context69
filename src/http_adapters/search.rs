use async_trait::async_trait;
use context69_search_http::SearchApi;

use crate::services::{auth::AuthService, query::QueryService};

#[derive(Clone)]
pub struct SearchApiAdapter {
    query: QueryService,
    auth: AuthService,
}

impl SearchApiAdapter {
    pub fn new(query: QueryService, auth: AuthService) -> Self {
        Self { query, auth }
    }
}

#[async_trait]
impl SearchApi for SearchApiAdapter {
    async fn search(
        &self,
        user_id: Option<i64>,
        request: crate::contracts::SearchRequest,
    ) -> anyhow::Result<crate::contracts::SearchResponse> {
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
