//! Manifest planning, configuration fingerprints, and chunk conversion.

use anyhow::Result;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::contracts::sources::GitRepositorySkipSummary;
use crate::db::{Database, NewGitGenerationChunk, StoredGitRepositoryGeneration};
use crate::services::git_repository::code_text::CodeChunk;

use super::super::incremental::BaselineManifest;
use super::super::limits::GitIndexLimits;
use super::super::selection::{SkipReason, SkipSummary};
use super::GIT_INDEX_PIPELINE_VERSION;

pub(super) fn plan_is_incremental(
    seeded: usize,
    baseline: Option<&StoredGitRepositoryGeneration>,
) -> bool {
    seeded > 0 || baseline.is_some()
}

pub(super) fn build_config_fingerprint(
    source: &crate::db::StoredGitRepositorySource,
    limits: &GitIndexLimits,
) -> String {
    let mut digest = Sha256::new();
    let parts = [
        GIT_INDEX_PIPELINE_VERSION.to_string(),
        format!("{:?}", source.index_profile),
        source.include_patterns.join("\u{1f}"),
        source.exclude_patterns.join("\u{1f}"),
        limits.chunk_bounds.max_lines.to_string(),
        limits.chunk_bounds.max_bytes.to_string(),
        limits.chunk_bounds.max_chunks.to_string(),
        limits.max_files.to_string(),
    ];
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) async fn load_baseline_manifest(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    generation_key: Uuid,
) -> Result<BaselineManifest> {
    let mut manifest = BaselineManifest::new();
    let mut offset = 0i64;
    loop {
        let page = db
            .list_git_generation_files(
                group_id,
                repository_key,
                generation_key,
                crate::db::MAX_GIT_LIST_PAGE,
                offset,
            )
            .await?;
        if page.is_empty() {
            break;
        }
        for file in &page {
            manifest.insert(
                file.path.clone(),
                (file.provider_blob_sha.clone(), file.byte_count),
            );
        }
        if (page.len() as i64) < crate::db::MAX_GIT_LIST_PAGE {
            break;
        }
        offset += page.len() as i64;
    }
    Ok(manifest)
}

pub(super) fn skip_summary_contract(
    skipped: &SkipSummary,
    binary: usize,
    too_large: usize,
) -> GitRepositorySkipSummary {
    GitRepositorySkipSummary {
        excluded_directory: count(skipped.count(SkipReason::ExcludedDirectory)),
        binary: count(skipped.count(SkipReason::Binary).saturating_add(binary)),
        minified: count(skipped.count(SkipReason::Minified)),
        source_map: count(skipped.count(SkipReason::SourceMap)),
        lockfile: count(skipped.count(SkipReason::Lockfile)),
        secret: count(skipped.count(SkipReason::Secret)),
        too_large: count(
            skipped
                .count(SkipReason::TooLarge)
                .saturating_add(too_large),
        ),
        excluded_by_user: count(skipped.count(SkipReason::ExcludedByUser)),
        not_included: count(skipped.count(SkipReason::NotIncluded)),
    }
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(super) fn chunk_rows(chunks: Vec<CodeChunk>) -> Vec<NewGitGenerationChunk> {
    chunks
        .into_iter()
        .map(|chunk| NewGitGenerationChunk {
            chunk_index: chunk.chunk_index,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            text: chunk.text,
        })
        .collect()
}

pub(super) fn bounded_count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
