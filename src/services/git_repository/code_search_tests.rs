//! Pure fusion, profile, fallback, and identity tests for the shared code search.
//!
//! These never touch a database, Qdrant, or a provider: they build stored rows and
//! vector hits directly, so the fixed RRF rule, deterministic ordering, profile
//! metadata, and destination identity are pinned without a live dependency.

use chrono::Utc;
use uuid::Uuid;

use super::*;
use crate::contracts::sources::{
    GitCommitCheckpoint, GitGenerationStatus, GitIndexStatus, GitProviderKind, GitRefreshPolicy,
    GitVersionPolicy,
};
use crate::db::{
    GitGroupOwnership, StoredGitGenerationCodeVectors, StoredGitRepositoryGeneration,
    StoredGitRepositorySource,
};
use crate::services::git_repository::code_vectors::{CodeVectorRequest, code_vector_target};
use crate::services::git_repository::support::{FakeCodeProvider, code_snapshot};

const GROUP_ID: i64 = 42;

fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

fn generation_key() -> Uuid {
    Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid")
}

fn chunk_key(suffix: u128) -> Uuid {
    Uuid::from_u128(suffix)
}

fn source(profile: GitIndexProfile) -> StoredGitRepositorySource {
    StoredGitRepositorySource {
        group: GitGroupOwnership {
            group_id: GROUP_ID,
            group_key: "team-platform".to_string(),
            group_path: "acme/team-platform".to_string(),
            visibility: crate::contracts::Visibility::Private,
        },
        repository_key: repository_key(),
        connection_key: None,
        provider: GitProviderKind::GitHub,
        canonical_url: "https://github.com/cloudiful/context69".to_string(),
        owner: "cloudiful".to_string(),
        name: "context69".to_string(),
        default_branch: "main".to_string(),
        version: GitVersionPolicy {
            ref_name: "refs/heads/main".to_string(),
            commit_sha: None,
        },
        refresh: GitRefreshPolicy::Manual,
        index_profile: profile,
        index_status: GitIndexStatus::Ready,
        checkpoint: GitCommitCheckpoint {
            target_commit_sha: None,
            indexed_commit_sha: None,
            indexed_at: None,
            checkpoint_updated_at: None,
        },
        active_generation_key: Some(generation_key()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn generation() -> StoredGitRepositoryGeneration {
    StoredGitRepositoryGeneration {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        index_profile: GitIndexProfile::Lexical,
        status: GitGenerationStatus::Ready,
        file_count: 3,
        excluded_file_count: 0,
        total_bytes: 128,
        error_code: None,
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn lexical_hit(path: &str, index: i32, score: f32) -> GitCodeLexicalHit {
    GitCodeLexicalHit {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        visibility: crate::contracts::Visibility::Private,
        file_key: chunk_key(1),
        path: path.to_string(),
        language: "rust".to_string(),
        chunk_key: chunk_key(index as u128),
        chunk_index: index,
        start_line: index,
        end_line: index,
        text: format!("text-{index}"),
        score,
        matched: GitCodeMatchKind::ChunkPhrase,
    }
}

fn hydration(path: &str, index: i32) -> StoredGitCodeChunkHydration {
    StoredGitCodeChunkHydration {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        visibility: crate::contracts::Visibility::Private,
        file_key: chunk_key(1),
        path: path.to_string(),
        language: "rust".to_string(),
        chunk_key: chunk_key(index as u128),
        chunk_index: index,
        start_line: index,
        end_line: index,
        text: format!("text-{index}"),
    }
}

fn vector_hit(index: i32, score: f32) -> CodeVectorQueryHit {
    CodeVectorQueryHit {
        chunk_key: chunk_key(index as u128),
        score,
    }
}

fn hydrated(
    rows: impl IntoIterator<Item = StoredGitCodeChunkHydration>,
) -> HashMap<Uuid, StoredGitCodeChunkHydration> {
    rows.into_iter().map(|row| (row.chunk_key, row)).collect()
}

#[test]
fn hybrid_uses_fixed_rrf_and_keeps_lexical_metadata_on_duplicates() {
    let lexical = vec![lexical_hit("a.rs", 1, 9.0), lexical_hit("b.rs", 2, 8.0)];
    let vector = vec![vector_hit(2, 0.99), vector_hit(3, 0.9)];
    let hydrated = hydrated([hydration("c.rs", 3)]);
    let (hits, truncated) = hybrid_result(&lexical, &vector, &hydrated, 20);

    assert!(!truncated);
    assert_eq!(hits.len(), 3, "the union deduplicates the shared chunk");
    // b.rs appears in both channels and keeps its lexical match kind.
    let duplicate = hits
        .iter()
        .find(|hit| hit.chunk_key == chunk_key(2))
        .expect("the duplicate survives");
    assert_eq!(duplicate.path, "b.rs");
    assert_eq!(duplicate.matched, GitCodeMatchKind::ChunkPhrase);
    let expected = 1.0 / (RRF_K + 2.0) + 1.0 / (RRF_K + 1.0);
    assert!((f64::from(duplicate.score) - expected).abs() < 1e-6);

    // Score desc → path asc on ties; each channel contributes 1/(60+rank).
    let scores: Vec<f32> = hits.iter().map(|hit| hit.score).collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|left, right| right.total_cmp(left));
    assert_eq!(scores, sorted, "fused scores are descending");
}

#[test]
fn hybrid_truncation_is_evidence_from_the_union() {
    let lexical = vec![lexical_hit("a.rs", 1, 9.0), lexical_hit("b.rs", 2, 8.0)];
    let vector = vec![vector_hit(3, 0.9), vector_hit(4, 0.8)];
    let hydrated = hydrated([hydration("c.rs", 3), hydration("d.rs", 4)]);
    let (hits, truncated) = hybrid_result(&lexical, &vector, &hydrated, 2);
    assert!(truncated, "the union exceeds the requested limit");
    assert_eq!(hits.len(), 2, "only the requested page is returned");
    let mut kept = hits.iter().map(|hit| hit.score).collect::<Vec<_>>();
    kept.sort_by(|left, right| right.total_cmp(left));
    assert_eq!(kept, hits.iter().map(|hit| hit.score).collect::<Vec<_>>());
}

#[test]
fn full_semantic_uses_cosine_scores_and_deterministic_ties() {
    let vector = vec![vector_hit(1, 0.7), vector_hit(2, 0.9), vector_hit(3, 0.9)];
    let hydrated = hydrated([
        hydration("a.rs", 1),
        hydration("b.rs", 2),
        hydration("c.rs", 3),
    ]);
    let (hits, truncated) = full_semantic_result(&vector, &hydrated, 20);
    assert!(!truncated);
    assert_eq!(hits.len(), 3, "an unhydrated key never becomes a text hit");
    assert!(
        hits.iter()
            .all(|hit| hit.matched == GitCodeMatchKind::Semantic)
    );
    assert!(hits.iter().all(|hit| hit.score > 0.0 && hit.score <= 0.9));
    // Equal cosine scores tie-break by path, then chunk index, then key.
    assert_eq!(hits[0].path, "b.rs");
    assert_eq!(hits[1].path, "c.rs");
    assert_eq!(hits[2].path, "a.rs");
}

#[test]
fn full_semantic_reports_a_dropped_orphan_as_truncated() {
    let vector = vec![vector_hit(1, 0.9), vector_hit(2, 0.8)];
    let hydrated = hydrated([hydration("a.rs", 1)]);
    let (hits, truncated) = full_semantic_result(&vector, &hydrated, 20);
    assert_eq!(hits.len(), 1);
    assert!(
        truncated,
        "a filtered-out candidate is evidence the page is incomplete"
    );
}

#[test]
fn lexical_outcome_preserves_order_scores_and_truncation() {
    let lexical = vec![
        lexical_hit("b.rs", 2, 8.0),
        lexical_hit("a.rs", 1, 9.0),
        lexical_hit("c.rs", 3, 7.0),
    ];
    let outcome = lexical_outcome(
        &source(GitIndexProfile::Lexical),
        &generation(),
        lexical,
        2,
        GitIndexProfile::Lexical,
        None,
    );
    assert_eq!(outcome.effective_profile, GitIndexProfile::Lexical);
    assert_eq!(outcome.vector_unavailable, None);
    assert!(outcome.truncated, "the third row is the truncation probe");
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        vec!["b.rs", "a.rs"],
        "lexical order and scores are untouched"
    );
    assert_eq!(outcome.hits[0].score, 8.0);
    assert_eq!(outcome.hits[1].score, 9.0);
}

#[test]
fn a_fallback_outcome_states_the_reason_and_effective_lexical() {
    let outcome = lexical_outcome(
        &source(GitIndexProfile::Hybrid),
        &generation(),
        vec![lexical_hit("a.rs", 1, 1.0)],
        20,
        GitIndexProfile::Hybrid,
        Some(GitCodeVectorUnavailableReason::IdentityMismatch),
    );
    assert_eq!(outcome.requested_profile, GitIndexProfile::Hybrid);
    assert_eq!(outcome.effective_profile, GitIndexProfile::Lexical);
    assert_eq!(
        outcome.vector_unavailable,
        Some(GitCodeVectorUnavailableReason::IdentityMismatch)
    );
}

#[test]
fn the_destination_match_requires_identity_collection_and_dimensions() {
    let snapshot = code_snapshot(FakeCodeProvider::new(8, false), 8, "fp-a");
    let target = code_vector_target(&CodeVectorRequest::production(
        snapshot,
        Some("http://qdrant.invalid:6334".to_string()),
    ))
    .expect("a complete request resolves a target");
    let metadata = StoredGitGenerationCodeVectors {
        generation_key: generation_key(),
        fingerprint: target.destination_fingerprint().to_string(),
        dimensions: 8,
        collection_name: target.collection_name().to_string(),
        point_count: 1,
        created_at: Utc::now(),
    };
    assert!(destination_matches(&metadata, &target));

    let mut wrong_fingerprint = metadata.clone();
    wrong_fingerprint.fingerprint = "fp-b".to_string();
    assert!(!destination_matches(&wrong_fingerprint, &target));

    let mut wrong_collection = metadata.clone();
    wrong_collection.collection_name = "context69_code_v1_other".to_string();
    assert!(!destination_matches(&wrong_collection, &target));

    let mut wrong_dimensions = metadata;
    wrong_dimensions.dimensions = 16;
    assert!(!destination_matches(&wrong_dimensions, &target));
}

#[test]
fn full_semantic_composes_overlapping_lexical_candidates_from_hydrated_rows() {
    let lexical = vec![lexical_hit("a.rs", 1, 9.0), lexical_hit("b.rs", 2, 8.0)];
    let vector = vec![vector_hit(2, 0.9), vector_hit(1, 0.8)];
    let keys = semantic_hydration_keys(&vector);
    assert_eq!(
        keys,
        vec![chunk_key(2), chunk_key(1)],
        "every vector candidate is selected for hydration, overlap included"
    );
    let hydrated = hydrated([hydration("a.rs", 1), hydration("b.rs", 2)]);

    let (hits, truncated) = assemble_semantic_result(
        GitIndexProfile::FullSemantic,
        &lexical,
        &vector,
        &hydrated,
        20,
    );
    assert!(!truncated, "all overlapping candidates hydrated");
    assert_eq!(hits.len(), 2, "both vector candidates are returned");
    assert!(
        hits.iter()
            .all(|hit| hit.matched == GitCodeMatchKind::Semantic)
    );
    assert_eq!(hits[0].path, "b.rs");
    assert_eq!(hits[0].score, 0.9);
    assert_eq!(hits[1].path, "a.rs");
    assert_eq!(hits[1].score, 0.8);
}

#[test]
fn full_semantic_mixed_overlap_and_vector_only_returns_each_once_and_drops_unauthorized() {
    let lexical = vec![lexical_hit("a.rs", 1, 9.0), lexical_hit("b.rs", 2, 8.0)];
    let vector = vec![vector_hit(2, 0.9), vector_hit(3, 0.8), vector_hit(4, 0.7)];
    assert_eq!(
        semantic_hydration_keys(&vector),
        vec![chunk_key(2), chunk_key(3), chunk_key(4)],
        "the vector-only key and the unauthorized key are both attempted"
    );
    // Key 4 matches no scoped row: it must never become text or a hit.
    let hydrated = hydrated([hydration("b.rs", 2), hydration("c.rs", 3)]);

    let (hits, truncated) = assemble_semantic_result(
        GitIndexProfile::FullSemantic,
        &lexical,
        &vector,
        &hydrated,
        20,
    );
    assert!(
        truncated,
        "an unhydratable candidate marks the page incomplete"
    );
    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_key).collect::<Vec<_>>(),
        vec![chunk_key(2), chunk_key(3)],
        "the overlap and the vector-only key are each returned once"
    );
    assert!(!hits.iter().any(|hit| hit.chunk_key == chunk_key(4)));
}

#[test]
fn hybrid_composition_keeps_duplicate_lexical_metadata_with_all_vector_keys_hydrated() {
    let lexical = vec![lexical_hit("a.rs", 1, 9.0), lexical_hit("b.rs", 2, 8.0)];
    let vector = vec![vector_hit(2, 0.9), vector_hit(3, 0.8)];
    let keys = semantic_hydration_keys(&vector);
    assert_eq!(keys, vec![chunk_key(2), chunk_key(3)]);
    let hydrated = hydrated([hydration("b.rs", 2), hydration("c.rs", 3)]);

    let (hits, truncated) =
        assemble_semantic_result(GitIndexProfile::Hybrid, &lexical, &vector, &hydrated, 20);
    assert!(!truncated);
    assert_eq!(hits.len(), 3, "the union returns each chunk once");
    let duplicate = hits
        .iter()
        .find(|hit| hit.chunk_key == chunk_key(2))
        .expect("the overlapping chunk survives");
    assert_eq!(duplicate.path, "b.rs");
    assert_eq!(duplicate.matched, GitCodeMatchKind::ChunkPhrase);
}
