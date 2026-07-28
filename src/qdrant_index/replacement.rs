use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use qdrant_client::qdrant::{
    GetPointsBuilder, PointId, PointStruct, RetrievedPoint, Vector, vector_output,
};
use tokio::time::timeout;
use uuid::Uuid;

use super::{QDRANT_OPERATION_TIMEOUT, QdrantIndex, point_id_to_uuid};
use crate::domain::ChunkPayload;

pub(crate) struct DocumentChunkReplacement {
    index: QdrantIndex,
    previous_points: Vec<PointStruct>,
    previous_point_ids: HashSet<Uuid>,
    replacement_chunk_ids: Vec<Uuid>,
}

impl DocumentChunkReplacement {
    pub(crate) async fn rollback(self) -> Result<()> {
        self.index
            .restore_points(
                &self.previous_points,
                &self.previous_point_ids,
                &self.replacement_chunk_ids,
            )
            .await
    }
}

impl QdrantIndex {
    pub(crate) async fn replace_document_chunks_with_rollback(
        &self,
        existing_chunk_ids: &[Uuid],
        payloads: &[ChunkPayload],
        embeddings: &[Vec<f32>],
    ) -> Result<DocumentChunkReplacement> {
        let points = self.build_document_points(payloads, embeddings)?;
        let replacement_chunk_ids = unique_ids(payloads.iter().map(|payload| payload.chunk_id));
        let replacement_ids = replacement_chunk_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        let (previous_points, previous_point_ids) =
            self.snapshot_points(existing_chunk_ids).await?;

        if let Err(error) = self.upsert_points(points, payloads.len()).await {
            return Err(self
                .error_after_restore(
                    error,
                    &previous_points,
                    &previous_point_ids,
                    &replacement_chunk_ids,
                )
                .await);
        }

        let obsolete_ids = existing_chunk_ids
            .iter()
            .copied()
            .filter(|id| !replacement_ids.contains(id))
            .collect::<Vec<_>>();
        if let Err(error) = self.delete_points(&obsolete_ids).await {
            return Err(self
                .error_after_restore(
                    error,
                    &previous_points,
                    &previous_point_ids,
                    &replacement_chunk_ids,
                )
                .await);
        }

        Ok(DocumentChunkReplacement {
            index: self.clone(),
            previous_points,
            previous_point_ids,
            replacement_chunk_ids,
        })
    }

    async fn snapshot_points(
        &self,
        chunk_ids: &[Uuid],
    ) -> Result<(Vec<PointStruct>, HashSet<Uuid>)> {
        if chunk_ids.is_empty() {
            return Ok((Vec::new(), HashSet::new()));
        }

        let ids = unique_ids(chunk_ids.iter().copied())
            .into_iter()
            .map(|id| PointId::from(id.to_string()))
            .collect::<Vec<_>>();
        let response = timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.get_points(
                GetPointsBuilder::new(&self.collection_name, ids)
                    .with_payload(true)
                    .with_vectors(true),
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "qdrant points snapshot request timed out after {}s",
                QDRANT_OPERATION_TIMEOUT.as_secs()
            )
        })?
        .context("qdrant points snapshot request failed")?;

        let mut point_ids = HashSet::with_capacity(response.result.len());
        let mut points = Vec::with_capacity(response.result.len());
        for point in response.result {
            let (chunk_id, point) = point_to_struct(point)?;
            point_ids.insert(chunk_id);
            points.push(point);
        }
        Ok((points, point_ids))
    }

    async fn error_after_restore(
        &self,
        error: anyhow::Error,
        previous_points: &[PointStruct],
        previous_point_ids: &HashSet<Uuid>,
        replacement_chunk_ids: &[Uuid],
    ) -> anyhow::Error {
        match self
            .restore_points(previous_points, previous_point_ids, replacement_chunk_ids)
            .await
        {
            Ok(()) => error,
            Err(restore_error) => {
                anyhow!("{error:#}; failed to restore previous Qdrant points: {restore_error:#}")
            }
        }
    }

    async fn restore_points(
        &self,
        previous_points: &[PointStruct],
        previous_point_ids: &HashSet<Uuid>,
        replacement_chunk_ids: &[Uuid],
    ) -> Result<()> {
        let mut errors = Vec::new();
        if !previous_points.is_empty()
            && let Err(error) = self
                .upsert_points(previous_points.to_vec(), previous_points.len())
                .await
        {
            errors.push(format!("restore previous points: {error:#}"));
        }

        let replacement_only_ids = replacement_chunk_ids
            .iter()
            .copied()
            .filter(|id| !previous_point_ids.contains(id))
            .collect::<Vec<_>>();
        if let Err(error) = self.delete_points(&replacement_only_ids).await {
            errors.push(format!("delete replacement-only points: {error:#}"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(errors.join("; ")))
        }
    }
}

fn point_to_struct(point: RetrievedPoint) -> Result<(Uuid, PointStruct)> {
    let point_id = point.id.context("missing qdrant point id in snapshot")?;
    let chunk_id = point_id_to_uuid(point_id)?;
    let vectors = point
        .vectors
        .context("missing qdrant vector in point snapshot")?;
    let vector = vectors
        .get_vector()
        .context("named qdrant vectors are not supported for rollback")?;
    let vector = match vector {
        vector_output::Vector::Dense(vector) => Vector::from(vector),
        vector_output::Vector::Sparse(vector) => Vector::from(vector),
        vector_output::Vector::MultiDense(vector) => Vector::from(vector),
    };
    Ok((chunk_id, PointStruct::new(chunk_id, vector, point.payload)))
}

fn unique_ids(ids: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}
