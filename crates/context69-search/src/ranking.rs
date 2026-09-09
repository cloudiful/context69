use std::collections::HashMap;

use context69_contracts::SearchHit;
use uuid::Uuid;

use crate::{CachedRerankItemScore, RerankHit};

#[derive(Debug)]
struct Candidate {
    hit: SearchHit,
}

/// Normalized hybrid fusion weights: `vector` and `keyword` come from
/// `SearchSettings`, and `boost` closes the unit budget so that
/// `vector + keyword + boost = 1`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FusionWeights {
    pub vector: f32,
    pub keyword: f32,
    pub boost: f32,
}

impl FusionWeights {
    pub(crate) fn new(vector: f32, keyword: f32) -> Self {
        let vector = vector.clamp(0.0, 1.0);
        // The keyword share may not exceed the budget the vector share leaves
        // behind. Settings validation enforces this already; the clamp only
        // keeps the unit-budget invariant for defensive callers.
        let keyword = keyword.clamp(0.0, (1.0 - vector).max(0.0));
        Self {
            vector,
            keyword,
            boost: (1.0 - vector - keyword).max(0.0),
        }
    }
}

pub(crate) fn merge_candidates(
    vector_results: Vec<SearchHit>,
    keyword_results: Vec<SearchHit>,
    query: &str,
    weights: FusionWeights,
) -> Vec<SearchHit> {
    let mut candidates: HashMap<(i64, i32), Candidate> = HashMap::new();

    for hit in vector_results {
        let key = (hit.document_id, hit.chunk_index);
        candidates
            .entry(key)
            .and_modify(|candidate| merge_hit(&mut candidate.hit, &hit))
            .or_insert(Candidate { hit });
    }

    for keyword_hit in keyword_results {
        candidates
            .entry((keyword_hit.document_id, keyword_hit.chunk_index))
            .and_modify(|candidate| {
                merge_hit(&mut candidate.hit, &keyword_hit);
            })
            .or_insert(Candidate { hit: keyword_hit });
    }

    candidates
        .into_values()
        .map(|mut candidate| {
            candidate.hit.score = local_score(&candidate.hit, query, weights);
            candidate.hit
        })
        .collect()
}

fn merge_hit(current: &mut SearchHit, incoming: &SearchHit) {
    let vector_score = max_score(current.vector_score, incoming.vector_score);
    let keyword_score = max_score(current.keyword_score, incoming.keyword_score);
    let rerank_score = max_score(current.rerank_score, incoming.rerank_score);
    let match_reason = current
        .match_reason
        .clone()
        .or_else(|| incoming.match_reason.clone());
    if current.is_fallback && !incoming.is_fallback {
        *current = incoming.clone();
    }
    current.vector_score = vector_score;
    current.keyword_score = keyword_score;
    current.rerank_score = rerank_score;
    current.match_reason = match_reason;
}

fn max_score(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

pub(crate) fn local_score(hit: &SearchHit, query: &str, weights: FusionWeights) -> f32 {
    let vector_score = hit.vector_score.unwrap_or(0.0).clamp(0.0, 1.0);
    // Keyword hits carry a trgm/cosine-like magnitude whose observed ceiling is
    // 2.37; dividing by 2.37 normalizes the channel onto [0, 1] for fusion.
    let keyword_score = (hit.keyword_score.unwrap_or(0.0).min(2.37) / 2.37).clamp(0.0, 1.0);
    let base = vector_score * weights.vector + keyword_score * weights.keyword;

    let query_lc = query.trim().to_lowercase();
    let title_lc = hit.title.to_lowercase();
    let chunk_lc = hit.chunk_text.to_lowercase();
    // Boost tiers multiply the configured boost weight: `weights.boost` is the
    // residual 1 - vector - keyword budget, so under the default weights (boost
    // share 0.10) the 1.8 / 1.4 / 1.0 tiers reproduce the legacy flat 0.18 /
    // 0.14 / 0.10 additions exactly, and rebalancing the fusion weights scales
    // the additions proportionally. The add stays capped by the residual unit
    // budget (1 - base), so fully saturated channel combinations reach the 1.0
    // ceiling while mid-range hits keep their spread.
    let boost_multiplier = if !query_lc.is_empty() && title_lc == query_lc {
        1.8
    } else if !query_lc.is_empty() && title_lc.contains(&query_lc) {
        1.4
    } else if !query_lc.is_empty() && chunk_lc.contains(&query_lc) {
        1.0
    } else {
        0.0
    };
    let boost_add = (boost_multiplier * weights.boost).min(1.0 - base);

    base + boost_add
}

pub(crate) fn compare_hits(left: &SearchHit, right: &SearchHit) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| right.published_at.cmp(&left.published_at))
        .then_with(|| left.document_id.cmp(&right.document_id))
        .then_with(|| left.chunk_index.cmp(&right.chunk_index))
}

pub(crate) fn rerank_document_text(hit: &SearchHit) -> String {
    format!(
        "标题: {}\n来源: {}\n日期: {}\n正文: {}",
        hit.title,
        hit.source_uri,
        hit.published_at
            .map(|date| date.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        hit.chunk_text
    )
}

pub(crate) fn apply_rerank(
    candidates: Vec<SearchHit>,
    rerank_candidates: &[SearchHit],
    reranked: &[RerankHit],
    requested_limit: usize,
) -> Vec<SearchHit> {
    // Map position in the stable `rerank_candidates` snapshot to the
    // relevance rank in the rerank output (its index in `reranked`) and the
    // score. The snapshot is the same regardless of the page offset, so
    // two requests sharing a query/filters/generation hit the same cache
    // entry and produce a consistent cross-page ordering.
    let mut position_to_score: HashMap<usize, f32> = HashMap::new();
    for hit in reranked {
        position_to_score.insert(hit.index, hit.score);
    }
    // Relevance rank: lower is better, assigned by position in `reranked`.
    // This is the key the final sort must use, not the snapshot position
    // (which would re-introduce the local ordering for items the rerank
    // already re-ordered).
    let mut position_to_rank: HashMap<usize, usize> = HashMap::new();
    for (rank, hit) in reranked.iter().enumerate() {
        position_to_rank.insert(hit.index, rank);
    }
    // Build chunk_id -> (relevance rank, rerank score) for snapshot items
    // that actually came back with a score. Items absent from the snapshot or
    // unranked fall back to the local hybrid ordering.
    let mut scored_snapshot: HashMap<Uuid, (usize, f32)> = HashMap::new();
    for (pos, snapshot_hit) in rerank_candidates.iter().enumerate() {
        if let (Some(rank), Some(score)) =
            (position_to_rank.get(&pos), position_to_score.get(&pos))
        {
            scored_snapshot.insert(snapshot_hit.chunk_id, (*rank, *score));
        }
    }

    // Annotate each candidate: scored items carry the rerank score and
    // the relevance rank, unscored items keep their local score.
    enum Order {
        Scored { rank: usize },
        Local,
    }
    let mut annotated: Vec<(SearchHit, Order)> = candidates
        .into_iter()
        .map(|mut hit| {
            if let Some(&(rank, score)) = scored_snapshot.get(&hit.chunk_id) {
                hit.rerank_score = Some(score);
                hit.score = score;
                (hit, Order::Scored { rank })
            } else {
                (hit, Order::Local)
            }
        })
        .collect();

    // Sort: scored items first, ordered by their relevance rank. Local
    // items follow in the existing deterministic hybrid order. The two
    // regions must not interleave or a page-2 slice could drop a page-1 row.
    annotated.sort_by(|left, right| match (&left.1, &right.1) {
        (Order::Scored { rank: l }, Order::Scored { rank: r }) => l.cmp(r),
        (Order::Scored { .. }, Order::Local) => std::cmp::Ordering::Less,
        (Order::Local, Order::Scored { .. }) => std::cmp::Ordering::Greater,
        (Order::Local, Order::Local) => compare_hits(&left.0, &right.0),
    });

    annotated
        .into_iter()
        .take(requested_limit)
        .map(|(hit, _)| hit)
        .collect()
}

pub(crate) fn merge_cached_item_scores(
    mut hot_scores: Vec<CachedRerankItemScore>,
    persisted_scores: Vec<CachedRerankItemScore>,
) -> Vec<CachedRerankItemScore> {
    for persisted in persisted_scores {
        if !hot_scores
            .iter()
            .any(|existing| existing.chunk_id == persisted.chunk_id)
        {
            hot_scores.push(persisted);
        }
    }
    hot_scores
}

#[cfg(test)]
mod tests {
    use context69_contracts::{SearchHit, Visibility};
    use serde_json::json;
    use uuid::Uuid;

    use crate::{CachedRerankItemScore, RerankHit};

    use super::{
        FusionWeights, apply_rerank, compare_hits, local_score, merge_cached_item_scores,
        merge_candidates,
    };

    fn default_weights() -> FusionWeights {
        FusionWeights::new(0.55, 0.35)
    }

    fn hit(chunk_id: Uuid, title: &str, chunk_text: &str) -> SearchHit {
        SearchHit {
            chunk_id,
            document_id: 1,
            group_key: "public".to_string(),
            group_path: "public".to_string(),
            visibility: Visibility::Public,
            source_key: "source".to_string(),
            external_id: "external".to_string(),
            title: title.to_string(),
            summary: None,
            source_uri: "https://example.com".to_string(),
            published_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .ok()
                .map(|value| value.with_timezone(&chrono::Utc)),
            chunk_index: 0,
            chunk_text: chunk_text.to_string(),
            score: 0.0,
            vector_score: None,
            keyword_score: None,
            rerank_score: None,
            match_reason: None,
            metadata_json: json!({}),
            library_file_id: None,
            library_section_label: None,
            library_path: None,
            is_library_file: false,
            requested_locale: None,
            content_locale: None,
            translation_status: None,
            is_fallback: false,
        }
    }

    #[test]
    fn local_keyword_score_boosts_exact_match() {
        let mut exact = hit(Uuid::new_v4(), "DeepSeek rollout", "policy text");
        exact.keyword_score = Some(1.0);
        let mut semantic = hit(Uuid::new_v4(), "AI rollout", "semantic policy text");
        semantic.vector_score = Some(0.50);

        assert!(
            local_score(&exact, "deepseek", default_weights())
                > local_score(&semantic, "deepseek", default_weights())
        );
    }

    #[test]
    fn default_boost_tiers_reproduce_legacy_flat_additions() {
        // Hits without channel scores have a zero base, isolating the boost
        // addition. Default boost share is 0.10 (1 - 0.55 - 0.35), so the
        // 1.8 / 1.4 / 1.0 multipliers must add 0.18 / 0.14 / 0.10 exactly.
        let exact_title = hit(Uuid::new_v4(), "DeepSeek", "unrelated body");
        let title_contains = hit(Uuid::new_v4(), "DeepSeek rollout", "unrelated body");
        let chunk_contains = hit(Uuid::new_v4(), "Unrelated title", "deepseek policy body");
        let no_match = hit(Uuid::new_v4(), "Unrelated title", "unrelated body");
        let weights = default_weights();
        assert!((weights.boost - 0.10).abs() < 1e-6);
        assert!(
            (local_score(&exact_title, "deepseek", weights) - 0.18).abs() < 1e-6,
            "exact title tier must add the legacy 0.18"
        );
        assert!(
            (local_score(&title_contains, "deepseek", weights) - 0.14).abs() < 1e-6,
            "title-contains tier must add the legacy 0.14"
        );
        assert!(
            (local_score(&chunk_contains, "deepseek", weights) - 0.10).abs() < 1e-6,
            "chunk-contains tier must add the legacy 0.10"
        );
        assert_eq!(local_score(&no_match, "deepseek", weights), 0.0);
    }

    #[test]
    fn rebalanced_boost_weight_scales_tiers_and_stays_capped() {
        // Move 0.10 of the vector share into the boost share: 0.10 -> 0.20.
        let rebalanced = FusionWeights::new(0.45, 0.35);
        assert!((rebalanced.boost - 0.20).abs() < 1e-6);

        let exact_title = hit(Uuid::new_v4(), "DeepSeek", "unrelated body");
        let title_contains = hit(Uuid::new_v4(), "DeepSeek rollout", "unrelated body");
        let chunk_contains = hit(Uuid::new_v4(), "Unrelated title", "deepseek policy body");

        // Doubling the boost share doubles the additions: 1.8 / 1.4 / 1.0 * 0.20.
        assert!((local_score(&exact_title, "deepseek", rebalanced) - 0.36).abs() < 1e-6);
        assert!((local_score(&title_contains, "deepseek", rebalanced) - 0.28).abs() < 1e-6);
        assert!((local_score(&chunk_contains, "deepseek", rebalanced) - 0.20).abs() < 1e-6);

        // Saturated channels still cap the add at the residual budget: base 0.80
        // (vector 1.0 + keyword 2.37) leaves 0.20, so the 0.36 exact-title add
        // is clamped and the total lands on the 1.0 ceiling.
        let mut hot = hit(Uuid::new_v4(), "DeepSeek", "deepseek policy body");
        hot.vector_score = Some(1.0);
        hot.keyword_score = Some(2.37);
        let score = local_score(&hot, "deepseek", rebalanced);
        assert!(score <= 1.0);
        assert!((score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fusion_weights_close_the_unit_budget() {
        let weights = default_weights();
        assert_eq!(weights.vector, 0.55);
        assert_eq!(weights.keyword, 0.35);
        // Boost closes the budget: w_vector + w_keyword + w_boost = 1.
        assert!((weights.vector + weights.keyword + weights.boost - 1.0).abs() < 1e-6);
        // A zero keyword channel hands its whole share to the boost margin.
        let weights = FusionWeights::new(0.7, 0.0);
        assert!((weights.vector + weights.keyword + weights.boost - 1.0).abs() < 1e-6);
        // Out-of-budget inputs are squeezed back into the unit budget.
        let weights = FusionWeights::new(0.9, 0.9);
        assert!(weights.vector + weights.keyword + weights.boost <= 1.0 + 1e-6);
    }

    #[test]
    fn local_score_never_needs_the_min_one_clamp() {
        // Maxed vector + keyword channels plus an exact title boost must land
        // on the unit ceiling without a min(1.0) truncation.
        let mut best = hit(Uuid::new_v4(), "DeepSeek", "DeepSeek policy text");
        best.vector_score = Some(1.0);
        best.keyword_score = Some(2.37);
        let score = local_score(&best, "deepseek", default_weights());
        assert!(score <= 1.0);
        assert!((score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn keyword_score_is_normalized_by_two_point_three_seven() {
        let mut hit = hit(Uuid::new_v4(), "unrelated title", "unrelated body text");
        hit.keyword_score = Some(2.37);
        hit.vector_score = None;
        // keyword 2.37 / 2.37 = 1.0 contributes exactly the keyword weight.
        assert!(
            (local_score(&hit, "query", default_weights()) - 0.35).abs() < 1e-6,
            "capped keyword score must contribute exactly the keyword weight"
        );
        // Scores above the 2.37 ceiling do not raise the contribution further.
        hit.keyword_score = Some(3.0);
        assert!(
            (local_score(&hit, "query", default_weights()) - 0.35).abs() < 1e-6,
            "keyword scores above the ceiling must stay capped"
        );
    }

    #[test]
    fn configured_weights_rebalance_channel_dominance() {
        let mut semantic = hit(Uuid::new_v4(), "Semantic title", "semantic body text");
        semantic.vector_score = Some(0.9);
        let mut keyword = hit(Uuid::new_v4(), "Keyword title", "keyword body text");
        keyword.keyword_score = Some(1.0);

        // Defaults favor a strong vector hit over a mid keyword hit.
        assert!(
            local_score(&semantic, "query", default_weights())
                > local_score(&keyword, "query", default_weights())
        );
        // Rebalancing toward keyword flips the order.
        let keyword_heavy = FusionWeights::new(0.15, 0.8);
        assert!(
            local_score(&keyword, "query", keyword_heavy)
                > local_score(&semantic, "query", keyword_heavy)
        );
        // Dropping the keyword weight to zero disables that channel entirely.
        let vector_only = FusionWeights::new(1.0, 0.0);
        assert_eq!(local_score(&keyword, "query", vector_only), 0.0);
        assert!(
            (local_score(&semantic, "query", vector_only) - 0.9).abs() < 1e-6
        );
    }

    #[test]
    fn merge_candidates_deduplicates_vector_and_keyword_hits() {
        let chunk_id = Uuid::new_v4();
        let mut vector = hit(chunk_id, "AI", "text");
        vector.vector_score = Some(0.5);
        let mut keyword = hit(chunk_id, "DeepSeek", "text");
        keyword.keyword_score = Some(1.0);
        keyword.match_reason = Some("title_phrase".to_string());

        let merged = merge_candidates(
            vec![vector],
            vec![keyword],
            "deepseek",
            default_weights(),
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].vector_score, Some(0.5));
        assert_eq!(merged[0].keyword_score, Some(1.0));
        assert_eq!(merged[0].match_reason.as_deref(), Some("title_phrase"));
    }

    #[test]
    fn merge_candidates_keeps_requested_locale_over_fallback_in_any_input_order() {
        for translated_first in [false, true] {
            let chunk_id = Uuid::new_v4();
            let mut translated = hit(chunk_id, "中文标题", "中文正文");
            translated.content_locale = Some("zh-CN".to_string());
            translated.requested_locale = Some("zh-CN".to_string());
            translated.keyword_score = Some(0.8);

            let mut fallback = hit(chunk_id, "English title", "English body");
            fallback.content_locale = Some("en-US".to_string());
            fallback.requested_locale = Some("zh-CN".to_string());
            fallback.is_fallback = true;
            fallback.vector_score = Some(0.7);

            let (vector, keyword) = if translated_first {
                (translated, fallback)
            } else {
                (fallback, translated)
            };
            let merged = merge_candidates(vec![vector], vec![keyword], "标题", default_weights());

            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].content_locale.as_deref(), Some("zh-CN"));
            assert!(!merged[0].is_fallback);
            assert_eq!(merged[0].vector_score, Some(0.7));
            assert_eq!(merged[0].keyword_score, Some(0.8));
        }
    }

    #[test]
    fn apply_rerank_orders_by_rerank_score() {
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let first = hit(first_id, "first", "first");
        let second = hit(second_id, "second", "second");

        let results = apply_rerank(
            vec![first, second.clone()],
            &[second.clone()],
            &[RerankHit {
                index: 0,
                score: 0.93,
            }],
            1,
        );

        assert_eq!(results[0].chunk_id, second_id);
        assert_eq!(results[0].rerank_score, Some(0.93));
    }

    #[test]
    fn apply_rerank_keeps_unscored_candidates_after_scored_region() {
        // The snapshot covers only the first three candidates; the fourth is
        // only present in the full set. The final ordering must keep all three
        // scored candidates in their rerank positions, then place the
        // unranked candidate at the end (local fallback).
        let a_id = Uuid::new_v4();
        let b_id = Uuid::new_v4();
        let c_id = Uuid::new_v4();
        let d_id = Uuid::new_v4();
        let a = hit(a_id, "a", "alpha");
        let b = hit(b_id, "b", "beta");
        let c = hit(c_id, "c", "gamma");
        let d = hit(d_id, "d", "delta");

        let snapshot = vec![a.clone(), b.clone(), c.clone()];
        let reranked = vec![
            RerankHit { index: 1, score: 0.9 }, // b
            RerankHit { index: 0, score: 0.7 }, // a
            RerankHit { index: 2, score: 0.5 }, // c
        ];

        let full = vec![a, b, c, d];
        let ordered = apply_rerank(full, &snapshot, &reranked, 4);
        assert_eq!(ordered[0].chunk_id, b_id);
        assert_eq!(ordered[1].chunk_id, a_id);
        assert_eq!(ordered[2].chunk_id, c_id);
        // Local fallback sits after the scored region.
        assert_eq!(ordered[3].chunk_id, d_id);
    }

    #[test]
    fn merge_cached_item_scores_prefers_existing_hot_entries() {
        let chunk_id = Uuid::new_v4();
        let merged = merge_cached_item_scores(
            vec![CachedRerankItemScore {
                chunk_id,
                score: 0.8,
            }],
            vec![CachedRerankItemScore {
                chunk_id,
                score: 0.5,
            }],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].score, 0.8);
    }

    #[test]
    fn compare_hits_breaks_score_ties_deterministically() {
        let mut older = hit(Uuid::new_v4(), "older", "text older");
        older.score = 0.5;
        older.document_id = 2;
        older.chunk_index = 1;
        older.published_at = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .ok()
            .map(|value| value.with_timezone(&chrono::Utc));
        let mut newer = hit(Uuid::new_v4(), "newer", "text newer");
        newer.score = 0.5;
        newer.document_id = 1;
        newer.chunk_index = 0;
        newer.published_at = chrono::DateTime::parse_from_rfc3339("2025-02-01T00:00:00Z")
            .ok()
            .map(|value| value.with_timezone(&chrono::Utc));

        // Newer published date wins when scores tie.
        assert_eq!(compare_hits(&older, &newer), std::cmp::Ordering::Greater);

        // Same score and date falls back to document/chunk order.
        newer.published_at = older.published_at;
        assert_eq!(compare_hits(&older, &newer), std::cmp::Ordering::Greater);
        assert_eq!(compare_hits(&newer, &older), std::cmp::Ordering::Less);
    }

    #[test]
    fn merged_sorted_order_does_not_depend_on_input_order() {
        fn scored(title: &str, document_id: i64, chunk_index: i32, score: f32) -> SearchHit {
            let mut item = hit(Uuid::new_v4(), title, "body text");
            item.document_id = document_id;
            item.chunk_index = chunk_index;
            item.score = score;
            item.vector_score = Some(score);
            item
        }

        let first = scored("first", 1, 0, 0.9);
        let second = scored("second", 2, 0, 0.7);
        let third = scored("third", 3, 0, 0.9);
        // `first` and `third` tie on score/date; document_id decides.
        let forward = {
            let mut merged = merge_candidates(
                vec![first.clone(), second.clone()],
                vec![third.clone()],
                "query",
                default_weights(),
            );
            merged.sort_by(compare_hits);
            merged
                .iter()
                .map(|item| item.document_id)
                .collect::<Vec<_>>()
        };
        let reverse = {
            let mut merged = merge_candidates(
                vec![second.clone(), first.clone()],
                vec![third.clone()],
                "query",
                default_weights(),
            );
            merged.sort_by(compare_hits);
            merged
                .iter()
                .map(|item| item.document_id)
                .collect::<Vec<_>>()
        };

        assert_eq!(forward, reverse);
        // Highest score first, tie broken by smaller document_id.
        assert_eq!(forward[0], 1);
        assert_eq!(forward[1], 3);
        assert_eq!(forward[2], 2);
    }
}
