use std::time::Instant;

use tracing::info;

/// Behavior-preserving instrumentation accumulated during one search pipeline
/// run.
///
/// Records per-segment wall-clock timings plus candidate counts, cache-hit
/// facts, and the request window. `log()` emits a single consolidated structured
/// log line that keeps the existing `elapsed_ms` field and adds the per-segment
/// detail. Nothing here can change ranking, scoring, limits, caching, or API
/// shapes.
#[derive(Debug)]
pub(crate) struct SearchProbe {
    started: Instant,
    pub(crate) response_cache_hit: bool,
    pub(crate) embed_cache_hit: bool,
    pub(crate) embed_elapsed_ms: u64,
    pub(crate) vector_elapsed_ms: u64,
    pub(crate) vector_candidate_count: usize,
    pub(crate) vector_limit: usize,
    pub(crate) hydrate_elapsed_ms: u64,
    pub(crate) hydrate_candidate_count: usize,
    pub(crate) keyword_elapsed_ms: u64,
    pub(crate) keyword_candidate_count: usize,
    pub(crate) keyword_limit: Option<usize>,
    pub(crate) merge_elapsed_ms: u64,
    pub(crate) merged_candidate_count: usize,
    pub(crate) rerank_elapsed_ms: u64,
    pub(crate) rerank_batch_cache_hit: bool,
    pub(crate) rerank_item_cache_hit: bool,
    pub(crate) rerank_top_n: usize,
    pub(crate) rerank_candidate_len: usize,
    pub(crate) fetch_limit: usize,
    pub(crate) offset: usize,
    pub(crate) limit: usize,
    pub(crate) has_more: Option<bool>,
    pub(crate) result_count: usize,
}

impl SearchProbe {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            response_cache_hit: false,
            embed_cache_hit: false,
            embed_elapsed_ms: 0,
            vector_elapsed_ms: 0,
            vector_candidate_count: 0,
            vector_limit: 0,
            hydrate_elapsed_ms: 0,
            hydrate_candidate_count: 0,
            keyword_elapsed_ms: 0,
            keyword_candidate_count: 0,
            keyword_limit: None,
            merge_elapsed_ms: 0,
            merged_candidate_count: 0,
            rerank_elapsed_ms: 0,
            rerank_batch_cache_hit: false,
            rerank_item_cache_hit: false,
            rerank_top_n: 0,
            rerank_candidate_len: 0,
            fetch_limit: 0,
            offset: 0,
            limit: 0,
            has_more: None,
            result_count: 0,
        }
    }

    pub(crate) fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub(crate) fn log(&self) {
        info!(
            response_cache_hit = self.response_cache_hit,
            embed_cache_hit = self.embed_cache_hit,
            embed_elapsed_ms = self.embed_elapsed_ms,
            vector_elapsed_ms = self.vector_elapsed_ms,
            vector_candidate_count = self.vector_candidate_count,
            vector_limit = self.vector_limit,
            hydrate_elapsed_ms = self.hydrate_elapsed_ms,
            hydrate_candidate_count = self.hydrate_candidate_count,
            keyword_elapsed_ms = self.keyword_elapsed_ms,
            keyword_candidate_count = self.keyword_candidate_count,
            keyword_limit = ?self.keyword_limit,
            merge_elapsed_ms = self.merge_elapsed_ms,
            merged_candidate_count = self.merged_candidate_count,
            rerank_elapsed_ms = self.rerank_elapsed_ms,
            rerank_batch_cache_hit = self.rerank_batch_cache_hit,
            rerank_item_cache_hit = self.rerank_item_cache_hit,
            rerank_top_n = self.rerank_top_n,
            rerank_candidate_len = self.rerank_candidate_len,
            fetch_limit = self.fetch_limit,
            offset = self.offset,
            limit = self.limit,
            has_more = ?self.has_more,
            result_count = self.result_count,
            elapsed_ms = self.elapsed_ms(),
            "search completed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::SearchProbe;

    #[test]
    fn defaults_report_no_cache_hits_and_zeroed_timings() {
        let probe = SearchProbe::new();
        assert!(!probe.response_cache_hit);
        assert!(!probe.embed_cache_hit);
        assert!(!probe.rerank_batch_cache_hit);
        assert!(!probe.rerank_item_cache_hit);
        assert_eq!(probe.vector_candidate_count, 0);
        assert_eq!(probe.keyword_candidate_count, 0);
        assert_eq!(probe.merged_candidate_count, 0);
        assert_eq!(probe.vector_limit, 0);
        assert_eq!(probe.fetch_limit, 0);
        assert_eq!(probe.keyword_limit, None);
        assert_eq!(probe.has_more, None);
        assert_eq!(probe.rerank_top_n, 0);
        assert_eq!(probe.rerank_candidate_len, 0);
    }

    #[test]
    fn elapsed_ms_does_not_decrease_with_wall_clock() {
        let probe = SearchProbe::new();
        let before = probe.elapsed_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let after = probe.elapsed_ms();
        assert!(after >= before);
    }
}
