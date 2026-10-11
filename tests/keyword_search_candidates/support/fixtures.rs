#[path = "cases.rs"]
mod cases;
#[path = "seed.rs"]
pub mod seed;

use context69::contracts::SearchRequest;
use context69::domain::AccessScope;

pub use cases::search_cases;

pub struct SeededFixture {
    pub tag: String,
    pub group_key: String,
    pub group_path: String,
    pub private_group_id: i64,
    pub private_group_key: String,
    pub private_group_path: String,
}

pub struct SearchCase {
    pub name: &'static str,
    pub request: SearchRequest,
    pub scope: AccessScope,
    pub limit: usize,
    pub expected_count: usize,
    pub expected_reason: Option<&'static str>,
}

pub fn request(query: impl Into<String>, group_path: &str) -> SearchRequest {
    SearchRequest {
        query: query.into(),
        locale: None,
        limit: 100,
        page: 1,
        source_key: None,
        group_path: Some(group_path.to_owned()),
        published_after: None,
        published_before: None,
        cursor: None,
        metadata_filters: Vec::new(),
        sort: context69::contracts::SearchSort::Relevance,
    }
}

pub fn public_scope() -> AccessScope {
    AccessScope {
        user_id: None,
        include_public: true,
        private_group_ids: Vec::new(),
        group_path: None,
        scoped_group_id: None,
    }
}
