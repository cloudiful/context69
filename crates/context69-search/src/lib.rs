mod abort;
mod cache;
mod ports;
mod ranking;
mod rerank;
mod service;
mod types;

pub use abort::{AbortOnDrop, AbortSignal, abort_pair};
pub use cache::{
    CachedRerankItemScore, SearchCache, hash_string, rerank_hits_from_item_scores,
    rerank_item_scores_complete, rerank_item_scores_from_hits, sort_cached_item_scores,
};
pub use ports::{
    DateBound, DateWindowPage, DateWindowQuery, SearchDatePointHit, SearchEmbeddingProvider,
    SearchIndex, SearchRepository, SearchScopeResolver,
};
pub use rerank::{OpenRouterRerankClient, RerankClient, RerankDocument, RerankHit};
pub use service::{SearchService, decode_search_cursor_for_validation};
pub use types::{AccessScope, SearchPointHit, SearchSettings, StoredRerankItemScore};