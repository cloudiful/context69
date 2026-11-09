//! Shared lexical/semantic/hybrid Git code search (issue 25 P3).
//!
//! One service owns admission (group Viewer floor, group-scoped repository,
//! captured active ready generation), the one code snapshot and destination, the
//! lexical and read-only vector channels, canonical hydration, fusion, and the
//! requested/effective profile metadata. HTTP and MCP adapters only validate
//! their own input and project this outcome.
//!
//! Lexical retrieval is unchanged: a `Lexical` repository returns exactly the
//! existing lexical hits and scores and never calls a code provider. A
//! `Hybrid`/`FullSemantic` repository whose code channel is absent, unrecorded,
//! mismatched, or unavailable falls back to the same lexical result with an
//! explicit reason. No document provider, secret, or collection is consulted.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use uuid::Uuid;

use crate::config::QdrantConfig;
use crate::contracts::MembershipRole;
use crate::contracts::sources::{
    GitCodeLexicalHit, GitCodeMatchKind, GitCodeSearchHit, GitCodeVectorUnavailableReason,
    GitIndexProfile,
};
use crate::db::{
    Database, GitLexicalCodeSearch, StoredGitCodeChunkHydration, StoredGitRepositoryGeneration,
    StoredGitRepositorySource,
};
use crate::domain::{AccessScope, GroupRecord};
use crate::domain_errors::DomainError;
use crate::qdrant_index::QdrantIndex;
use crate::qdrant_index::SearchPointHit;
use crate::services::settings::SettingsService;

use super::code_vectors::{CodeVectorRequest, CodeVectorTarget, code_vector_target};

/// Fixed Reciprocal Rank Fusion constant.
pub(crate) const RRF_K: f64 = 60.0;

/// One validated search request.
pub(crate) struct CodeSearchParams<'a> {
    pub repository_key: Uuid,
    pub query: &'a str,
    pub path_prefix: Option<&'a str>,
    pub language: Option<&'a str>,
    pub limit: i64,
}

/// The shared result the HTTP and MCP adapters project.
pub(crate) struct CodeSearchOutcome {
    pub source: StoredGitRepositorySource,
    pub generation: StoredGitRepositoryGeneration,
    pub hits: Vec<GitCodeSearchHit>,
    pub truncated: bool,
    pub requested_profile: GitIndexProfile,
    pub effective_profile: GitIndexProfile,
    pub vector_unavailable: Option<GitCodeVectorUnavailableReason>,
}

/// Runs one admission-checked code search.
pub(crate) async fn search_code(
    db: &Database,
    settings: &SettingsService,
    group: &GroupRecord,
    scope: &AccessScope,
    params: &CodeSearchParams<'_>,
) -> Result<CodeSearchOutcome> {
    require_viewer(group)?;
    let source = db
        .get_git_repository_source(group.id, params.repository_key)
        .await?
        .ok_or_else(|| anyhow!(DomainError::not_found("unknown git repository")))?;
    let generation = active_ready_generation(db, &source)
        .await?
        .ok_or_else(|| anyhow!(DomainError::conflict("no active ready index generation")))?;
    search_captured(db, settings, &source, &generation, scope, params).await
}

fn require_viewer(group: &GroupRecord) -> Result<()> {
    let allowed = group
        .current_role
        .is_some_and(|role| role.rank() >= MembershipRole::Viewer.rank());
    if allowed {
        Ok(())
    } else {
        Err(DomainError::forbidden("insufficient permissions for group").into())
    }
}

async fn active_ready_generation(
    db: &Database,
    source: &StoredGitRepositorySource,
) -> Result<Option<StoredGitRepositoryGeneration>> {
    let group_id = source.group.group_id;
    let Some(active) = db
        .get_git_active_generation(group_id, source.repository_key)
        .await?
    else {
        return Ok(None);
    };
    let Some(generation) = db
        .get_git_repository_generation(group_id, source.repository_key, active.generation_key)
        .await?
    else {
        return Ok(None);
    };
    Ok(
        (generation.status == crate::contracts::sources::GitGenerationStatus::Ready)
            .then_some(generation),
    )
}

async fn search_captured(
    db: &Database,
    settings: &SettingsService,
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    scope: &AccessScope,
    params: &CodeSearchParams<'_>,
) -> Result<CodeSearchOutcome> {
    let requested = source.index_profile;
    let lexical = lexical_channel(db, source, scope, params).await?;
    if requested == GitIndexProfile::Lexical {
        return Ok(lexical_outcome(
            source,
            generation,
            lexical,
            params.limit,
            requested,
            None,
        ));
    }

    let channel = vector_channel(db, settings, source, generation, params).await?;
    let Some(unavailable) = channel.unavailable else {
        // Every vector candidate is hydrated from the captured generation,
        // including keys the lexical channel also returned. A FullSemantic
        // answer must never drop a valid overlapping candidate, and the scoped
        // statement still authorizes each row against the captured generation,
        // owning group, current visibility, path, and language.
        let vector_hits = channel.hits;
        let hydration_keys = semantic_hydration_keys(&vector_hits);
        let hydrated = hydrate(db, source, generation, scope, params, &hydration_keys).await?;

        // Any activation change during the query discards the captured mix and
        // answers once from the canonical current lexical result.
        let Some(current) = active_ready_generation(db, source).await? else {
            return Err(DomainError::conflict("no active ready index generation").into());
        };
        if current.generation_key != generation.generation_key {
            let lexical = lexical_channel(db, source, scope, params).await?;
            return Ok(lexical_outcome(
                source,
                &current,
                lexical,
                params.limit,
                requested,
                Some(GitCodeVectorUnavailableReason::IdentityMismatch),
            ));
        }

        let (hits, truncated) =
            assemble_semantic_result(requested, &lexical, &vector_hits, &hydrated, params.limit);
        return Ok(CodeSearchOutcome {
            source: source.clone(),
            generation: generation.clone(),
            hits,
            truncated,
            requested_profile: requested,
            effective_profile: requested,
            vector_unavailable: None,
        });
    };

    // Fallback keeps the existing lexical result and states the one reason.
    let Some(current) = active_ready_generation(db, source).await? else {
        return Err(DomainError::conflict("no active ready index generation").into());
    };
    if current.generation_key != generation.generation_key {
        let lexical = lexical_channel(db, source, scope, params).await?;
        return Ok(lexical_outcome(
            source,
            &current,
            lexical,
            params.limit,
            requested,
            Some(unavailable),
        ));
    }
    Ok(lexical_outcome(
        source,
        generation,
        lexical,
        params.limit,
        requested,
        Some(unavailable),
    ))
}

/// One vector-channel attempt: either scored hits or the explicit reason the
/// channel could not run. Database failures stay errors and never become a
/// lexical success.
struct VectorChannel {
    hits: Vec<SearchPointHit>,
    unavailable: Option<GitCodeVectorUnavailableReason>,
}

async fn vector_channel(
    db: &Database,
    settings: &SettingsService,
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    params: &CodeSearchParams<'_>,
) -> Result<VectorChannel> {
    let Some(snapshot) = settings.code_embedding_snapshot() else {
        return Ok(unavailable(GitCodeVectorUnavailableReason::NotConfigured));
    };
    let Some(qdrant_url) = settings.runtime_qdrant_url().await? else {
        return Ok(unavailable(GitCodeVectorUnavailableReason::NotConfigured));
    };
    if qdrant_url.trim().is_empty() {
        return Ok(unavailable(GitCodeVectorUnavailableReason::NotConfigured));
    }
    let request = CodeVectorRequest::production(snapshot, Some(qdrant_url));
    let Ok(target) = code_vector_target(&request) else {
        return Ok(unavailable(GitCodeVectorUnavailableReason::NotConfigured));
    };
    let Some(metadata) = db
        .get_git_generation_code_vectors(
            source.group.group_id,
            source.repository_key,
            generation.generation_key,
        )
        .await?
    else {
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::GenerationNotVectorized,
        ));
    };
    if !destination_matches(&metadata, &target) {
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::IdentityMismatch,
        ));
    }
    let Ok(dimensions) = usize::try_from(metadata.dimensions) else {
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::IdentityMismatch,
        ));
    };
    if dimensions == 0 {
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::IdentityMismatch,
        ));
    }

    let Ok(query_vector) = target.snapshot().provider.embed_query(params.query).await else {
        tracing::warn!(
            repository_key = %source.repository_key,
            generation_key = %generation.generation_key,
            reason = "embed_query_failed",
            "code semantic channel unavailable"
        );
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::RuntimeUnavailable,
        ));
    };
    if query_vector.len() != dimensions || !query_vector.iter().all(|value| value.is_finite()) {
        tracing::warn!(
            repository_key = %source.repository_key,
            generation_key = %generation.generation_key,
            reason = "query_vector_invalid",
            "code semantic channel unavailable"
        );
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::RuntimeUnavailable,
        ));
    }

    let config = QdrantConfig {
        url: target.qdrant_url().to_string(),
        collection_name: metadata.collection_name.clone(),
        recreate_on_dimension_mismatch: false,
    };
    let Ok(index) = QdrantIndex::attach_code_read_only_collection(
        &config,
        &metadata.collection_name,
        dimensions,
    )
    .await
    else {
        tracing::warn!(
            repository_key = %source.repository_key,
            generation_key = %generation.generation_key,
            reason = "collection_unavailable",
            "code semantic channel unavailable"
        );
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::RuntimeUnavailable,
        ));
    };
    let Ok(mut hits) = index
        .search_code_vectors(
            source.group.group_id,
            source.repository_key,
            generation.generation_key,
            &query_vector,
            params.limit + 1,
        )
        .await
    else {
        tracing::warn!(
            repository_key = %source.repository_key,
            generation_key = %generation.generation_key,
            reason = "search_failed",
            "code semantic channel unavailable"
        );
        return Ok(unavailable(
            GitCodeVectorUnavailableReason::RuntimeUnavailable,
        ));
    };
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    Ok(VectorChannel {
        hits,
        unavailable: None,
    })
}

fn unavailable(reason: GitCodeVectorUnavailableReason) -> VectorChannel {
    VectorChannel {
        hits: Vec::new(),
        unavailable: Some(reason),
    }
}

fn destination_matches(
    metadata: &crate::db::StoredGitGenerationCodeVectors,
    target: &CodeVectorTarget,
) -> bool {
    metadata.fingerprint == target.destination_fingerprint()
        && metadata.collection_name == target.collection_name()
        && usize::try_from(metadata.dimensions)
            .map(|dimensions| dimensions == target.dimensions())
            .unwrap_or(false)
}

async fn lexical_channel(
    db: &Database,
    source: &StoredGitRepositorySource,
    scope: &AccessScope,
    params: &CodeSearchParams<'_>,
) -> Result<Vec<GitCodeLexicalHit>> {
    db.lexical_search_git_generation_chunks(
        source.group.group_id,
        &GitLexicalCodeSearch {
            repository_key: source.repository_key,
            query: params.query.to_string(),
            path_prefix: params.path_prefix.map(str::to_string),
            language: params.language.map(str::to_string),
            visible_group_ids: scope.private_group_ids.clone(),
            limit: params.limit + 1,
        },
    )
    .await
}

async fn hydrate(
    db: &Database,
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    scope: &AccessScope,
    params: &CodeSearchParams<'_>,
    keys: &[Uuid],
) -> Result<HashMap<Uuid, StoredGitCodeChunkHydration>> {
    let rows = db
        .hydrate_git_generation_chunks_by_keys(
            source.group.group_id,
            source.repository_key,
            generation.generation_key,
            keys,
            params.path_prefix,
            params.language,
            &scope.private_group_ids,
        )
        .await?;
    Ok(rows.into_iter().map(|row| (row.chunk_key, row)).collect())
}

pub(crate) fn lexical_outcome(
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    lexical: Vec<GitCodeLexicalHit>,
    limit: i64,
    requested_profile: GitIndexProfile,
    unavailable: Option<GitCodeVectorUnavailableReason>,
) -> CodeSearchOutcome {
    let truncated = lexical.len() as i64 > limit;
    let hits = lexical
        .iter()
        .take(limit.max(0) as usize)
        .map(search_hit_from_lexical)
        .collect::<Vec<_>>();
    CodeSearchOutcome {
        source: source.clone(),
        generation: generation.clone(),
        hits,
        truncated,
        requested_profile,
        effective_profile: GitIndexProfile::Lexical,
        vector_unavailable: if requested_profile == GitIndexProfile::Lexical {
            None
        } else {
            unavailable
        },
    }
}

/// Keys whose canonical stored rows must be hydrated for one semantic answer.
///
/// Every vector candidate is included even when the lexical channel returned the
/// same chunk: a FullSemantic result is assembled only from hydrated canonical
/// rows, so excluding an overlapping key would silently drop a valid candidate.
/// The scoped hydration statement still revalidates generation, group,
/// visibility, path, and language for every key.
pub(crate) fn semantic_hydration_keys(vector_hits: &[SearchPointHit]) -> Vec<Uuid> {
    vector_hits.iter().map(|hit| hit.chunk_id).collect()
}

/// Assemble the channel result for the requested profile.
///
/// This is the exact production composition step used by `search_captured`:
/// FullSemantic scores only hydrated vector candidates with cosine scores and a
/// semantic match kind; Hybrid keeps lexical projections on duplicates and
/// fuses both channels with fixed RRF.
fn assemble_semantic_result(
    requested: GitIndexProfile,
    lexical: &[GitCodeLexicalHit],
    vector_hits: &[SearchPointHit],
    hydrated: &HashMap<Uuid, StoredGitCodeChunkHydration>,
    limit: i64,
) -> (Vec<GitCodeSearchHit>, bool) {
    if requested == GitIndexProfile::FullSemantic {
        full_semantic_result(vector_hits, hydrated, limit)
    } else {
        hybrid_result(lexical, vector_hits, hydrated, limit)
    }
}

fn full_semantic_result(
    vector: &[SearchPointHit],
    hydrated: &HashMap<Uuid, StoredGitCodeChunkHydration>,
    limit: i64,
) -> (Vec<GitCodeSearchHit>, bool) {
    let kept = vector
        .iter()
        .filter_map(|hit| {
            hydrated
                .get(&hit.chunk_id)
                .map(|row| search_hit_from_hydration(row, hit.score, GitCodeMatchKind::Semantic))
        })
        .collect::<Vec<_>>();
    let dropped = vector
        .iter()
        .filter(|hit| !hydrated.contains_key(&hit.chunk_id))
        .count();
    let mut ordered = kept;
    sort_hits(&mut ordered);
    ordered.truncate(limit.max(0) as usize);
    (ordered, vector.len() as i64 > limit || dropped > 0)
}

fn hybrid_result(
    lexical: &[GitCodeLexicalHit],
    vector: &[SearchPointHit],
    hydrated: &HashMap<Uuid, StoredGitCodeChunkHydration>,
    limit: i64,
) -> (Vec<GitCodeSearchHit>, bool) {
    let mut lexical_rank: HashMap<Uuid, usize> = HashMap::new();
    let mut lexical_by_key: HashMap<Uuid, &GitCodeLexicalHit> = HashMap::new();
    for (index, hit) in lexical.iter().enumerate() {
        lexical_rank.entry(hit.chunk_key).or_insert(index + 1);
        lexical_by_key.entry(hit.chunk_key).or_insert(hit);
    }
    let mut vector_rank: HashMap<Uuid, usize> = HashMap::new();
    for (index, hit) in vector.iter().enumerate() {
        vector_rank.entry(hit.chunk_id).or_insert(index + 1);
    }

    let mut keys: Vec<Uuid> = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    for hit in lexical.iter().map(|hit| hit.chunk_key).chain(
        vector
            .iter()
            .map(|hit| hit.chunk_id)
            .filter(|key| !lexical_by_key.contains_key(key)),
    ) {
        if seen.insert(hit) {
            keys.push(hit);
        }
    }

    let dropped = vector
        .iter()
        .filter(|hit| {
            !lexical_by_key.contains_key(&hit.chunk_id) && !hydrated.contains_key(&hit.chunk_id)
        })
        .count();
    let mut hits = keys
        .into_iter()
        .filter_map(|key| {
            let mut score = 0.0_f64;
            if let Some(rank) = lexical_rank.get(&key) {
                score += 1.0 / (RRF_K + *rank as f64);
            }
            if let Some(rank) = vector_rank.get(&key) {
                score += 1.0 / (RRF_K + *rank as f64);
            }
            lexical_by_key.get(&key).map(|hit| {
                let mut projected = search_hit_from_lexical(hit);
                projected.score = score as f32;
                projected
            })
        })
        .chain(hydrated.values().filter_map(|row| {
            if lexical_by_key.contains_key(&row.chunk_key) {
                return None;
            }
            let rank = vector_rank.get(&row.chunk_key)?;
            let score = 1.0 / (RRF_K + *rank as f64);
            Some(search_hit_from_hydration(
                row,
                score as f32,
                GitCodeMatchKind::Semantic,
            ))
        }))
        .collect::<Vec<_>>();
    sort_hits(&mut hits);
    let union_len = hits.len();
    hits.truncate(limit.max(0) as usize);
    (
        hits,
        lexical.len() as i64 > limit
            || vector.len() as i64 > limit
            || union_len as i64 > limit
            || dropped > 0,
    )
}

fn sort_hits(hits: &mut [GitCodeSearchHit]) {
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.chunk_index.cmp(&right.chunk_index))
            .then_with(|| left.chunk_key.cmp(&right.chunk_key))
    });
}

pub(crate) fn search_hit_from_lexical(hit: &GitCodeLexicalHit) -> GitCodeSearchHit {
    GitCodeSearchHit {
        repository_key: hit.repository_key,
        generation_key: hit.generation_key,
        generation_number: hit.generation_number,
        ref_name: hit.ref_name.clone(),
        commit_sha: hit.commit_sha.clone(),
        visibility: hit.visibility,
        file_key: hit.file_key,
        path: hit.path.clone(),
        language: hit.language.clone(),
        chunk_key: hit.chunk_key,
        chunk_index: hit.chunk_index,
        start_line: hit.start_line,
        end_line: hit.end_line,
        text: hit.text.clone(),
        score: hit.score,
        matched: hit.matched,
    }
}

fn search_hit_from_hydration(
    row: &StoredGitCodeChunkHydration,
    score: f32,
    matched: GitCodeMatchKind,
) -> GitCodeSearchHit {
    GitCodeSearchHit {
        repository_key: row.repository_key,
        generation_key: row.generation_key,
        generation_number: row.generation_number,
        ref_name: row.ref_name.clone(),
        commit_sha: row.commit_sha.clone(),
        visibility: row.visibility,
        file_key: row.file_key,
        path: row.path.clone(),
        language: row.language.clone(),
        chunk_key: row.chunk_key,
        chunk_index: row.chunk_index,
        start_line: row.start_line,
        end_line: row.end_line,
        text: row.text.clone(),
        score,
        matched,
    }
}

#[cfg(test)]
#[path = "code_search_tests.rs"]
mod tests;
