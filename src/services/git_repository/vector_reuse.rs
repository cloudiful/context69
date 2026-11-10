//! Bounded vector reuse for incremental Git generations (issue #28 phase P2).
//!
//! An unchanged file whose baseline generation was embedded into the same
//! destination can copy its vectors instead of re-embedding its text. Only
//! exact path/blob/chunk-index pairs qualify, the baseline and target are both
//! confined to the same group and repository, and every read and write stays on
//! one bounded page. A reused point that the destination no longer holds falls
//! back to embedding that chunk, so a partial baseline never drops content.

use std::collections::HashMap;

use anyhow::Result;
use uuid::Uuid;

use crate::db::{Database, GitChunkEmbeddingRow, GitChunkReuseRow, MAX_GIT_LIST_PAGE};
use crate::qdrant_index::code_vectors::CodeVectorPoint;

use super::code_vectors::{CodeVectorSink, CodeVectorTarget, validate_embedding_vectors};

/// Copies every reusable baseline vector and embeds the remaining chunks.
///
/// Returns the total acknowledged point count of the target generation.
pub(crate) async fn copy_and_embed_generation(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    baseline_generation_key: Uuid,
    target_generation_key: Uuid,
    target: &CodeVectorTarget,
    sink: &dyn CodeVectorSink,
) -> Result<i64> {
    let mut acknowledged = 0i64;
    acknowledged += copy_reusable_vectors(
        db,
        group_id,
        repository_key,
        baseline_generation_key,
        target_generation_key,
        target,
        sink,
    )
    .await?;
    acknowledged += embed_unmatched_chunks(
        db,
        group_id,
        repository_key,
        baseline_generation_key,
        target_generation_key,
        target,
        sink,
    )
    .await?;
    Ok(acknowledged)
}

async fn copy_reusable_vectors(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    baseline_generation_key: Uuid,
    target_generation_key: Uuid,
    target: &CodeVectorTarget,
    sink: &dyn CodeVectorSink,
) -> Result<i64> {
    let dimensions = target.dimensions();
    let mut acknowledged = 0i64;
    let mut offset = 0i64;
    loop {
        let page = db
            .list_git_generation_chunk_reuse_map(
                group_id,
                repository_key,
                target_generation_key,
                baseline_generation_key,
                MAX_GIT_LIST_PAGE,
                offset,
            )
            .await?;
        if page.is_empty() {
            break;
        }
        let source_keys = page
            .iter()
            .map(|row| row.source_chunk_key)
            .collect::<Vec<_>>();
        let stored = sink.retrieve(&source_keys).await?;
        let vectors: HashMap<Uuid, Vec<f32>> = stored.into_iter().collect();
        let mut points = Vec::new();
        let mut missing = Vec::new();
        for row in &page {
            match vectors.get(&row.source_chunk_key) {
                Some(vector)
                    if vector.len() == dimensions
                        && vector.iter().all(|value| value.is_finite()) =>
                {
                    points.push(reuse_point(
                        row,
                        vector.clone(),
                        group_id,
                        repository_key,
                        target_generation_key,
                    ));
                }
                _ => missing.push(row),
            }
        }
        if !points.is_empty() {
            sink.write(dimensions, &points).await?;
            acknowledged = acknowledged.saturating_add(bounded_count(points.len()));
        }
        if !missing.is_empty() {
            let embedded = embed_pages(
                &missing
                    .iter()
                    .map(|row| row.chunk_text.clone())
                    .collect::<Vec<_>>(),
                target,
            )
            .await?;
            let mut points = Vec::with_capacity(missing.len());
            for (row, vector) in missing.iter().zip(embedded) {
                points.push(reuse_point(
                    row,
                    vector,
                    group_id,
                    repository_key,
                    target_generation_key,
                ));
            }
            sink.write(dimensions, &points).await?;
            acknowledged = acknowledged.saturating_add(bounded_count(points.len()));
        }
        if (page.len() as i64) < MAX_GIT_LIST_PAGE {
            break;
        }
        offset += page.len() as i64;
    }
    Ok(acknowledged)
}

async fn embed_unmatched_chunks(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    baseline_generation_key: Uuid,
    target_generation_key: Uuid,
    target: &CodeVectorTarget,
    sink: &dyn CodeVectorSink,
) -> Result<i64> {
    let dimensions = target.dimensions();
    let mut acknowledged = 0i64;
    let mut offset = 0i64;
    loop {
        let page = db
            .list_git_generation_chunks_needing_embedding(
                group_id,
                repository_key,
                target_generation_key,
                baseline_generation_key,
                MAX_GIT_LIST_PAGE,
                offset,
            )
            .await?;
        if page.is_empty() {
            break;
        }
        let texts = page
            .iter()
            .map(|row| row.chunk_text.clone())
            .collect::<Vec<_>>();
        let vectors = embed_pages(&texts, target).await?;
        let points = page
            .iter()
            .zip(vectors)
            .map(|(row, vector)| {
                embedding_point(row, vector, group_id, repository_key, target_generation_key)
            })
            .collect::<Vec<_>>();
        sink.write(dimensions, &points).await?;
        acknowledged = acknowledged.saturating_add(bounded_count(points.len()));
        if (page.len() as i64) < MAX_GIT_LIST_PAGE {
            break;
        }
        offset += page.len() as i64;
    }
    Ok(acknowledged)
}

async fn embed_pages(texts: &[String], target: &CodeVectorTarget) -> Result<Vec<Vec<f32>>> {
    let vectors = target.snapshot().provider.embed_texts(texts).await?;
    validate_embedding_vectors(&vectors, texts.len(), target.dimensions())?;
    Ok(vectors)
}

fn reuse_point(
    row: &GitChunkReuseRow,
    vector: Vec<f32>,
    group_id: i64,
    repository_key: Uuid,
    generation_key: Uuid,
) -> CodeVectorPoint {
    CodeVectorPoint {
        chunk_key: row.target_chunk_key,
        vector,
        group_id,
        repository_key,
        generation_key,
        file_key: row.target_file_key,
        chunk_index: row.chunk_index,
        path: row.path.clone(),
        language: row.language.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
    }
}

fn embedding_point(
    row: &GitChunkEmbeddingRow,
    vector: Vec<f32>,
    group_id: i64,
    repository_key: Uuid,
    generation_key: Uuid,
) -> CodeVectorPoint {
    CodeVectorPoint {
        chunk_key: row.chunk_key,
        vector,
        group_id,
        repository_key,
        generation_key,
        file_key: row.file_key,
        chunk_index: row.chunk_index,
        path: row.path.clone(),
        language: row.language.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
    }
}

fn bounded_count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
