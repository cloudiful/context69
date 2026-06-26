use std::collections::HashMap;

use context69_contracts::SearchHit;
use uuid::Uuid;

use crate::{CachedRerankItemScore, RerankHit};

#[derive(Debug)]
struct Candidate {
    hit: SearchHit,
}

pub(crate) fn merge_candidates(
    vector_results: Vec<SearchHit>,
    keyword_results: Vec<SearchHit>,
    query: &str,
) -> Vec<SearchHit> {
    let mut candidates: HashMap<Uuid, Candidate> = HashMap::new();

    for hit in vector_results {
        candidates.insert(hit.chunk_id, Candidate { hit });
    }

    for keyword_hit in keyword_results {
        candidates
            .entry(keyword_hit.chunk_id)
            .and_modify(|candidate| {
                candidate.hit.keyword_score = keyword_hit.keyword_score;
                candidate.hit.match_reason = keyword_hit.match_reason.clone();
            })
            .or_insert(Candidate { hit: keyword_hit });
    }

    candidates
        .into_values()
        .map(|mut candidate| {
            candidate.hit.score = local_score(&candidate.hit, query);
            candidate.hit
        })
        .collect()
}

pub(crate) fn local_score(hit: &SearchHit, query: &str) -> f32 {
    let vector_score = hit.vector_score.unwrap_or(0.0).clamp(0.0, 1.0);
    let keyword_score = hit.keyword_score.unwrap_or(0.0).min(2.37) / 2.37;
    let query_lc = query.trim().to_lowercase();
    let title_lc = hit.title.to_lowercase();
    let chunk_lc = hit.chunk_text.to_lowercase();
    let boost = if !query_lc.is_empty() && title_lc == query_lc {
        0.18
    } else if !query_lc.is_empty() && title_lc.contains(&query_lc) {
        0.14
    } else if !query_lc.is_empty() && chunk_lc.contains(&query_lc) {
        0.10
    } else {
        0.0
    };

    (vector_score * 0.55 + keyword_score * 0.35 + boost).min(1.0)
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
    mut candidates: Vec<SearchHit>,
    reranked: Vec<RerankHit>,
    requested_limit: usize,
) -> Vec<SearchHit> {
    let mut output = Vec::new();
    for result in reranked {
        if result.index >= candidates.len() {
            continue;
        }
        let mut hit = candidates[result.index].clone();
        hit.rerank_score = Some(result.score);
        hit.score = result.score;
        output.push(hit);
    }

    if output.len() < requested_limit {
        let used = output.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>();
        candidates.sort_by(compare_hits);
        for hit in candidates {
            if output.len() >= requested_limit {
                break;
            }
            if !used.contains(&hit.chunk_id) {
                output.push(hit);
            }
        }
    }

    output.truncate(requested_limit);
    output
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

    use super::{apply_rerank, local_score, merge_cached_item_scores, merge_candidates};

    fn hit(chunk_id: Uuid, title: &str, chunk_text: &str) -> SearchHit {
        SearchHit {
            chunk_id,
            document_id: 1,
            group_key: "public".to_string(),
            project_key: "default-public".to_string(),
            visibility: Visibility::Public,
            source_key: "source".to_string(),
            external_id: "external".to_string(),
            title: title.to_string(),
            summary: None,
            source_uri: "https://example.com".to_string(),
            published_at: chrono::NaiveDate::from_ymd_opt(2025, 1, 1),
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
        }
    }

    #[test]
    fn local_keyword_score_boosts_exact_match() {
        let mut exact = hit(Uuid::new_v4(), "DeepSeek rollout", "policy text");
        exact.keyword_score = Some(1.0);
        let mut semantic = hit(Uuid::new_v4(), "AI rollout", "semantic policy text");
        semantic.vector_score = Some(0.50);

        assert!(local_score(&exact, "deepseek") > local_score(&semantic, "deepseek"));
    }

    #[test]
    fn merge_candidates_deduplicates_vector_and_keyword_hits() {
        let chunk_id = Uuid::new_v4();
        let mut vector = hit(chunk_id, "AI", "text");
        vector.vector_score = Some(0.5);
        let mut keyword = hit(chunk_id, "DeepSeek", "text");
        keyword.keyword_score = Some(1.0);
        keyword.match_reason = Some("title_phrase".to_string());

        let merged = merge_candidates(vec![vector], vec![keyword], "deepseek");

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].vector_score, Some(0.5));
        assert_eq!(merged[0].keyword_score, Some(1.0));
        assert_eq!(merged[0].match_reason.as_deref(), Some("title_phrase"));
    }

    #[test]
    fn apply_rerank_orders_by_rerank_score() {
        let first = hit(Uuid::new_v4(), "first", "first");
        let second = hit(Uuid::new_v4(), "second", "second");

        let results = apply_rerank(
            vec![first, second.clone()],
            vec![RerankHit {
                index: 1,
                score: 0.93,
            }],
            1,
        );

        assert_eq!(results[0].chunk_id, second.chunk_id);
        assert_eq!(results[0].rerank_score, Some(0.93));
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
}
