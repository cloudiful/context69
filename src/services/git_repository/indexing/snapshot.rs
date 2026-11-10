//! One fenced snapshot attempt: acquisition, content writes, vectors, and
//! publication.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use bytes::Bytes;
use chrono::Utc;
use uuid::Uuid;

use crate::contracts::sources::{GitGenerationStatus, GitIndexProfile};
use crate::db::{
    Database, GitGenerationCoverage, GitGenerationPublication, NewGitGenerationChunk,
    NewGitGenerationCodeVectors, NewGitGenerationFile, NewGitRepositoryGeneration,
    StoredGitRepositorySource,
};
use crate::domain_errors::DomainError;

use super::super::classify::language_for_path;
use super::super::code_text::{CodeText, chunk_code_text_for_language};
use super::super::code_vectors::{
    CodeVectorRequest, CodeVectorSink, QdrantCodeVectorSink, code_vector_target,
    embed_and_write_generation,
};
use super::super::incremental::plan_incremental_snapshot;
use super::super::limits::{GitIndexLimits, plan_snapshot_files};
use super::super::ref_path_safety::{SafeRef, SafeSha};
use super::super::selection::{SelectionRules, select_snapshot_files};
use super::super::vector_reuse::copy_and_embed_generation;
use super::GIT_INDEX_PIPELINE_VERSION;
use super::GitSnapshotIndexOutcome;
use super::GitSnapshotProvider;
use super::manifest::{
    bounded_count, build_config_fingerprint, chunk_rows, load_baseline_manifest,
    plan_is_incremental, skip_summary_contract,
};
use super::rollback::SnapshotProgress;

/// Largest unchanged-file batch one seed statement copies.
const SEED_BATCH_FILES: usize = 500;

pub(super) async fn attempt_snapshot(
    db: &Database,
    provider: &dyn GitSnapshotProvider,
    limits: &GitIndexLimits,
    group_id: i64,
    repository_key: Uuid,
    code: Option<&CodeVectorRequest>,
    force_rebuild: bool,
    expected_config_revision: i64,
    progress: &mut SnapshotProgress,
) -> Result<GitSnapshotIndexOutcome> {
    let source = db
        .get_git_repository_source(group_id, repository_key)
        .await?
        .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
    if source.config_revision != expected_config_revision {
        progress.stale = true;
        return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
    }
    let config_revision = expected_config_revision;
    let observed_target = source.checkpoint.target_commit_sha.clone();
    let observed_force_rebuild = source.force_rebuild_requested_at;
    let force_rebuild = force_rebuild || observed_force_rebuild.is_some();
    progress.config_revision = Some(config_revision);
    let ref_name = source.version.ref_name.clone();
    let rules = SelectionRules::new(
        source.include_patterns.clone(),
        source.exclude_patterns.clone(),
    );
    let config_fingerprint = build_config_fingerprint(&source, limits);

    // A fresh acquirer owns a fresh aggregate byte budget for this generation.
    let acquirer = provider.acquirer(&source.canonical_url, limits.acquisition)?;

    // Resolve and pin the ref before any blob byte is spent. `commit_sha` is the
    // configured pin only; a branch target is resolved again on every run, so a
    // previously observed branch head never becomes a fixed pin.
    let pinned = match source.version.commit_sha.as_deref() {
        Some(commit) => SafeSha::parse(commit)?,
        None => acquirer.resolve_ref(&SafeRef::parse(&ref_name)?).await?,
    };

    let begun = db
        .begin_git_repository_index(
            group_id,
            repository_key,
            config_revision,
            observed_target.as_deref(),
            pinned.as_str(),
        )
        .await?;
    if !begun {
        progress.stale = true;
        return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
    }
    progress.indexing_marked = true;

    let requested_target = code
        .filter(|_| source.index_profile != GitIndexProfile::Lexical)
        .map(code_vector_target)
        .transpose()?;

    let baseline = if force_rebuild {
        None
    } else {
        let active = db
            .get_git_active_generation(group_id, repository_key)
            .await?;
        let baseline = match active {
            Some(active) => {
                db.get_git_repository_generation(group_id, repository_key, active.generation_key)
                    .await?
            }
            None => None,
        };
        let identity_matches = source.indexed_pipeline_version == GIT_INDEX_PIPELINE_VERSION
            && source.indexed_config_fingerprint == config_fingerprint;
        baseline.filter(|generation| {
            identity_matches && generation.status == GitGenerationStatus::Ready
        })
    };
    let baseline_code_reusable = match (&baseline, &requested_target) {
        (Some(generation), Some(target)) => db
            .get_git_generation_code_vectors(group_id, repository_key, generation.generation_key)
            .await?
            .is_some_and(|metadata| target.matches_metadata(&metadata)),
        _ => false,
    };

    // Same target and same pipeline/configuration identity: the active snapshot
    // already answers this request. Refresh the checkout and status timestamps
    // without writing a generation or re-embedding anything. A force rebuild
    // never takes this path.
    if !force_rebuild
        && let Some(generation) = &baseline
        && generation.commit_sha == pinned.as_str()
        && generation.ref_name == ref_name
        && (requested_target.is_none() || baseline_code_reusable)
    {
        let completed = db
            .complete_git_repository_noop(
                group_id,
                repository_key,
                config_revision,
                Some(pinned.as_str()),
                pinned.as_str(),
                GIT_INDEX_PIPELINE_VERSION,
                &config_fingerprint,
                &source.last_skip_summary,
            )
            .await?;
        if !completed {
            progress.stale = true;
            return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
        }
        progress.noop = true;
        return Ok(GitSnapshotIndexOutcome {
            repository_key,
            generation_key: generation.generation_key,
            generation_number: generation.generation_number,
            ref_name,
            commit_sha: pinned.as_str().to_owned(),
            file_count: generation.file_count,
            excluded_file_count: generation.excluded_file_count,
            total_bytes: generation.total_bytes,
            activated_at: generation.completed_at.unwrap_or_else(Utc::now),
            pipeline_version: GIT_INDEX_PIPELINE_VERSION,
            no_op: true,
            reused_file_count: generation.file_count,
            changed_file_count: 0,
            skip_summary: source.last_skip_summary,
        });
    }

    let generation = db
        .start_git_repository_generation(
            group_id,
            repository_key,
            &NewGitRepositoryGeneration {
                ref_name: ref_name.clone(),
                commit_sha: pinned.as_str().to_owned(),
                index_profile: source.index_profile,
            },
        )
        .await?;
    progress.generation_key = Some(generation.generation_key);

    let listing = acquirer.list_tree(&pinned).await?;
    let mut excluded_file_count = listing.excluded_files;
    let acquisition_too_large = listing.too_large_files;
    // Eligibility is decided from the listing before any blob byte is spent.
    // Safety exclusions and the per-file ceiling cannot be re-enabled by a user
    // pattern; user include/exclude rules are layered on top of the default
    // allowlist.
    let selection = select_snapshot_files(listing.files, &rules);
    excluded_file_count = excluded_file_count.saturating_add(selection.skipped.total());
    let target_entries = plan_snapshot_files(selection.files, limits)?;

    let baseline_manifest = match &baseline {
        Some(generation) => Some(
            load_baseline_manifest(db, group_id, repository_key, generation.generation_key).await?,
        ),
        None => None,
    };
    let plan = baseline_manifest
        .map(|manifest| plan_incremental_snapshot(target_entries.clone(), &manifest));
    let (seed_entries, fetch_entries, reused_file_count, changed_file_count) = match plan {
        Some(plan) => {
            let reused = plan.reused_file_count();
            let changed = plan.changed_file_count();
            (plan.seeded, plan.fetched, reused, changed)
        }
        None => {
            let changed = target_entries.len();
            (Vec::new(), target_entries, 0, changed)
        }
    };

    let mut total_bytes: i64 = seed_entries.iter().map(|entry| entry.byte_count).sum();
    let mut files: Vec<NewGitGenerationFile> = Vec::with_capacity(fetch_entries.len());
    let mut chunks_by_path: Vec<(String, Vec<NewGitGenerationChunk>)> = Vec::new();
    let mut blob_cache: HashMap<String, Bytes> = HashMap::new();
    let mut post_fetch_binary = 0usize;
    let mut post_fetch_too_large = 0usize;

    for entry in fetch_entries {
        let blob_key = entry.blob_sha.as_str().to_owned();
        let bytes = match blob_cache.get(&blob_key) {
            Some(cached) => cached.clone(),
            None => {
                let blob = acquirer.fetch_blob(&entry.blob_sha).await?;
                blob_cache.insert(blob_key.clone(), blob.bytes.clone());
                blob.bytes
            }
        };
        // Binary or NUL-carrying content cannot be stored as exact code text, so
        // the file is excluded and counted instead of failing the snapshot.
        let Ok(text) = CodeText::validate(bytes.as_ref()) else {
            excluded_file_count += 1;
            post_fetch_binary += 1;
            continue;
        };
        let language = language_for_path(entry.path.as_str());
        let Ok(chunks) = chunk_code_text_for_language(language, text.as_str(), limits.chunk_bounds)
        else {
            excluded_file_count += 1;
            post_fetch_too_large += 1;
            continue;
        };
        let byte_count = i64::try_from(bytes.len())
            .map_err(|_| anyhow!(DomainError::payload_too_large("git_file_too_large")))?;
        total_bytes = total_bytes.saturating_add(byte_count);
        files.push(NewGitGenerationFile {
            path: entry.path.as_str().to_owned(),
            language: language.to_owned(),
            provider_blob_sha: blob_key,
            content: bytes.to_vec(),
            line_count: text.line_count(),
        });
        chunks_by_path.push((entry.path.as_str().to_owned(), chunk_rows(chunks)));
    }

    let file_count = seed_entries.len().saturating_add(files.len());
    let coverage = GitGenerationCoverage {
        file_count: bounded_count(file_count),
        excluded_file_count: bounded_count(excluded_file_count),
        total_bytes,
    };

    let seeded = seed_entries.len();
    if seeded > 0 {
        let baseline_generation_key = baseline
            .as_ref()
            .map(|generation| generation.generation_key)
            .ok_or_else(|| anyhow!(DomainError::internal("git_index_baseline_missing")))?;
        for batch in seed_entries.chunks(SEED_BATCH_FILES) {
            let paths = batch
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>();
            let blob_shas = batch
                .iter()
                .map(|entry| entry.provider_blob_sha.clone())
                .collect::<Vec<_>>();
            db.seed_git_generation_files(
                group_id,
                repository_key,
                generation.generation_key,
                baseline_generation_key,
                &paths,
                &blob_shas,
            )
            .await?;
        }
    }

    if plan_is_incremental(seeded, baseline.as_ref()) {
        db.insert_git_generation_files(group_id, repository_key, generation.generation_key, &files)
            .await?;
    } else {
        db.replace_git_generation_files(
            group_id,
            repository_key,
            generation.generation_key,
            &files,
        )
        .await?;
    }

    for (path, chunks) in &chunks_by_path {
        let stored = db
            .get_git_generation_file(group_id, repository_key, generation.generation_key, path)
            .await?
            .ok_or_else(|| anyhow!(DomainError::internal("git_index_manifest_entry_missing")))?;
        db.replace_git_file_chunks(
            group_id,
            repository_key,
            generation.generation_key,
            stored.file_key,
            chunks,
        )
        .await?;
    }

    // The configuration revision and observed target must still be the ones
    // this attempt captured; a concurrent change is stale and must never
    // activate over it. Publication repeats this check under its own lock, so
    // this read is only an early exit.
    let current = db
        .get_git_repository_source(group_id, repository_key)
        .await?
        .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
    if !source_is_current(&current, config_revision, pinned.as_str()) {
        progress.stale = true;
        return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
    }

    // A non-lexical source with a configured code runtime writes canonical chunk
    // vectors before metadata and activation. A reusable baseline destination
    // copies unchanged vectors and embeds only changed/new chunks; every other
    // generation embeds its stored chunks. A lexical source never enters this
    // path, and a non-lexical source without a code runtime stays lexical
    // without metadata.
    if let Some(target) = requested_target
        && let Some(request) = code
    {
        let dimensions = i64::try_from(target.dimensions())
            .map_err(|_| anyhow!(DomainError::internal("git_code_dimensions_out_of_range")))?;
        // Ownership of the destination is recorded before the first remote
        // write. A later failure or stale target therefore keeps the only
        // reference to any points already acknowledged; the final count is
        // written only after every point is acknowledged.
        db.record_git_generation_code_vectors(
            group_id,
            repository_key,
            generation.generation_key,
            &NewGitGenerationCodeVectors {
                fingerprint: target.destination_fingerprint().to_string(),
                endpoint_identity: target.endpoint_identity().to_string(),
                dimensions,
                collection_name: target.collection_name().to_string(),
                point_count: 0,
            },
        )
        .await?;
        progress.code_vectors_recorded = true;
        let sink: Arc<dyn CodeVectorSink> = match request.test_sink() {
            Some(sink) => sink,
            None => Arc::new(QdrantCodeVectorSink::open(&target).await?),
        };
        let point_count = match (&baseline, baseline_code_reusable) {
            (Some(baseline_generation), true) => {
                copy_and_embed_generation(
                    db,
                    group_id,
                    repository_key,
                    baseline_generation.generation_key,
                    generation.generation_key,
                    &target,
                    sink.as_ref(),
                )
                .await?
            }
            _ => {
                embed_and_write_generation(
                    db,
                    group_id,
                    repository_key,
                    generation.generation_key,
                    &target,
                    sink.as_ref(),
                )
                .await?
            }
        };

        // The embedding pass can be long; the captured configuration revision
        // and observed target must still be current before metadata or
        // activation is written.
        let current = db
            .get_git_repository_source(group_id, repository_key)
            .await?
            .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
        if !source_is_current(&current, config_revision, pinned.as_str()) {
            progress.stale = true;
            return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
        }

        db.record_git_generation_code_vectors(
            group_id,
            repository_key,
            generation.generation_key,
            &NewGitGenerationCodeVectors {
                fingerprint: target.destination_fingerprint().to_string(),
                endpoint_identity: target.endpoint_identity().to_string(),
                dimensions,
                collection_name: target.collection_name().to_string(),
                point_count,
            },
        )
        .await?;
    }

    let skip_summary = skip_summary_contract(
        &selection.skipped,
        post_fetch_binary,
        post_fetch_too_large.saturating_add(acquisition_too_large),
    );

    // Completion, activation, and the checkpoint advance land in one statement,
    // fenced on the captured revision and observed target, so a newer edit can
    // never be overwritten and no window exists where only some of that state
    // advanced.
    let publication = GitGenerationPublication {
        expected_config_revision: config_revision,
        observed_commit_sha: pinned.as_str().to_owned(),
        pipeline_version: GIT_INDEX_PIPELINE_VERSION,
        config_fingerprint,
        skip_summary,
        coverage,
        observed_force_rebuild_requested_at: observed_force_rebuild,
    };
    let activated = db
        .publish_git_repository_generation(
            group_id,
            repository_key,
            generation.generation_key,
            &publication,
        )
        .await?;
    let Some(activated) = activated else {
        progress.stale = true;
        return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
    };
    progress.activated = true;

    Ok(GitSnapshotIndexOutcome {
        repository_key,
        generation_key: generation.generation_key,
        generation_number: generation.generation_number,
        ref_name,
        commit_sha: pinned.as_str().to_owned(),
        file_count: coverage.file_count,
        excluded_file_count: coverage.excluded_file_count,
        total_bytes: coverage.total_bytes,
        activated_at: activated.activated_at,
        pipeline_version: GIT_INDEX_PIPELINE_VERSION,
        no_op: false,
        reused_file_count: bounded_count(reused_file_count),
        changed_file_count: bounded_count(changed_file_count),
        skip_summary,
    })
}

/// Whether the source still matches the revision and observed target an attempt
/// captured. The revision covers target-ref, pin, profile, filter, enable, and
/// connection edits; the observed target covers another attempt advancing it.
fn source_is_current(
    source: &StoredGitRepositorySource,
    config_revision: i64,
    observed_commit_sha: &str,
) -> bool {
    source.config_revision == config_revision
        && source.enabled
        && source.deletion_requested_at.is_none()
        && source.checkpoint.target_commit_sha.as_deref() == Some(observed_commit_sha)
}
