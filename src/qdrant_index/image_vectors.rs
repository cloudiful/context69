//! Image point writes and deletions in the independent image collection.

use anyhow::{Result, anyhow};
use qdrant_client::{
    Payload,
    qdrant::{Condition, DeletePointsBuilder, Filter, PointId, PointStruct, PointsIdsList},
};
use tokio::time::timeout;
use uuid::Uuid;

use super::{
    QDRANT_OPERATION_TIMEOUT, QdrantIndex, format_qdrant_error, is_qdrant_idempotent_not_found,
    qdrant_timeout_error,
};
use crate::image_embedding_vectors::{ImageVectorPoint, image_vector_payload};

/// Shape/ownership marker version of the physical image collection.
pub(crate) const IMAGE_COLLECTION_SHAPE_VERSION: &str = "image-v1";

const IMAGE_METADATA_SHAPE_KEY: &str = "context69_image_shape";
const IMAGE_METADATA_FINGERPRINT_KEY: &str = "context69_image_runtime_fingerprint";
const IMAGE_METADATA_COLLECTION_KEY: &str = "context69_image_collection";

/// The collection-level ownership metadata written atomically by the create
/// request. Pinned qdrant-client 1.20.0 exposes `CreateCollection.metadata`
/// and returns it on `CollectionInfo.config.metadata`, so an interrupted
/// create can never leave an unowned collection behind: the collection either
/// does not exist or already carries this metadata.
pub(crate) fn image_collection_metadata(
    collection: &str,
    runtime_fingerprint: &str,
) -> std::collections::HashMap<String, serde_json::Value> {
    std::collections::HashMap::from([
        (
            IMAGE_METADATA_SHAPE_KEY.to_string(),
            serde_json::Value::String(IMAGE_COLLECTION_SHAPE_VERSION.to_string()),
        ),
        (
            IMAGE_METADATA_FINGERPRINT_KEY.to_string(),
            serde_json::Value::String(runtime_fingerprint.to_string()),
        ),
        (
            IMAGE_METADATA_COLLECTION_KEY.to_string(),
            serde_json::Value::String(collection.to_string()),
        ),
    ])
}

/// The persisted ownership metadata of one image collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageCollectionOwnership {
    pub(crate) shape_version: String,
    pub(crate) runtime_fingerprint: String,
    pub(crate) collection: String,
}

impl ImageCollectionOwnership {
    pub(crate) fn matches(&self, collection: &str, runtime_fingerprint: &str) -> bool {
        self.shape_version == IMAGE_COLLECTION_SHAPE_VERSION
            && self.runtime_fingerprint == runtime_fingerprint
            && self.collection == collection
    }
}

/// Reads the ownership metadata returned by a collection info response.
pub(crate) fn image_collection_ownership_from_metadata(
    metadata: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
) -> Option<ImageCollectionOwnership> {
    let read = |key: &str| {
        metadata
            .get(key)
            .map(|value| serde_json::Value::from(value.clone()))
            .and_then(|value| value.as_str().map(str::to_string))
    };
    Some(ImageCollectionOwnership {
        shape_version: read(IMAGE_METADATA_SHAPE_KEY)?,
        runtime_fingerprint: read(IMAGE_METADATA_FINGERPRINT_KEY)?,
        collection: read(IMAGE_METADATA_COLLECTION_KEY)?,
    })
}

/// The exact group and image-record conditions of one image search.
///
/// Every read carries the group scope and the image record marker; an optional
/// media filter only narrows that set.
pub(crate) fn image_search_filter(group_id: i64, media_types: &[String]) -> Filter {
    let mut must = vec![
        Condition::matches("group_id", group_id),
        Condition::matches("record_type", "image".to_string()),
    ];
    if !media_types.is_empty() {
        must.push(Condition::matches("media_type", media_types.to_vec()));
    }
    Filter {
        must,
        ..Default::default()
    }
}

/// Builds the Qdrant points, rejecting a wrong dimension or a non-finite value
/// before any request is issued.
fn build_image_points(points: &[ImageVectorPoint], dimensions: usize) -> Result<Vec<PointStruct>> {
    let mut built = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        if point.vector.len() != dimensions {
            return Err(anyhow!(
                "image embedding dimension mismatch at point {index}: expected {dimensions}, got \
                 {}",
                point.vector.len()
            ));
        }
        if !point.vector.iter().all(|value| value.is_finite()) {
            return Err(anyhow!(
                "image embedding contains a non-finite value at point {index}"
            ));
        }
        built.push(PointStruct::new(
            point.point_id.to_string(),
            point.vector.clone(),
            Payload::try_from(image_vector_payload(point))?,
        ));
    }
    Ok(built)
}

impl QdrantIndex {
    /// Search the independent image collection for one group scope.
    ///
    /// The caller has already attached a validated read-only image collection;
    /// the filter always carries the group and image record marker.
    pub(crate) async fn search_image_points(
        &self,
        group_id: i64,
        media_types: &[String],
        vector: Vec<f32>,
        limit: u64,
    ) -> Result<Vec<super::SearchPointHit>> {
        self.search_point_ids(
            "search_image_points",
            vector,
            limit,
            image_search_filter(group_id, media_types),
        )
        .await
    }

    /// Upsert one bounded page of image points with `wait=true`.
    ///
    /// The point id is the content revision and runtime identity, so a new
    /// revision or runtime writes its own point and the old point is deleted by
    /// exact id instead of being overwritten.
    pub(crate) async fn upsert_image_points(&self, points: &[ImageVectorPoint]) -> Result<()> {
        if points.is_empty() || self.is_noop() {
            return Ok(());
        }
        let built = build_image_points(points, self.dimensions)?;
        let operation = "upsert_image_points";
        let collection = self.collection_name.clone();
        let extra = format!("point_count={}", points.len());
        timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.upsert_points(
                qdrant_client::qdrant::UpsertPointsBuilder::new(&self.collection_name, built)
                    .wait(true),
            ),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra))?
        .map_err(|error| format_qdrant_error(operation, &collection, &extra, error.into()))?;
        Ok(())
    }

    /// Delete one exact image point from this collection.
    ///
    /// The id is the recorded revision-aware point id, so a delete can never
    /// reach another revision, another file, or newer content. A missing point
    /// is idempotent success.
    pub(crate) async fn delete_image_point(&self, point_id: Uuid) -> Result<()> {
        if self.is_noop() {
            return Ok(());
        }
        let operation = "delete_image_point";
        let collection = self.collection_name.clone();
        let extra = format!("point_id={point_id}");
        let result = timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.delete_points(
                DeletePointsBuilder::new(&self.collection_name)
                    .points(PointsIdsList {
                        ids: vec![PointId::from(point_id.to_string())],
                    })
                    .wait(true),
            ),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra));
        match result {
            Err(error) => Err(error),
            Ok(Ok(value)) => Ok(value).map(|_| ()),
            Ok(Err(error)) => {
                let error: anyhow::Error = error.into();
                if is_qdrant_idempotent_not_found(&error) {
                    return Ok(());
                }
                Err(format_qdrant_error(operation, &collection, &extra, error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IMAGE_COLLECTION_SHAPE_VERSION, build_image_points, image_collection_metadata,
        image_collection_ownership_from_metadata, image_search_filter,
    };
    use crate::image_embedding_vectors::ImageVectorPoint;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn point(vector: Vec<f32>) -> ImageVectorPoint {
        let file_id = Uuid::new_v4();
        let sha256 = "abc".to_string();
        let runtime_fingerprint = "fingerprint".to_string();
        let publish_attempt_id = Uuid::new_v4();
        ImageVectorPoint {
            point_id: crate::image_embedding_vectors::image_point_id(
                file_id,
                &sha256,
                &runtime_fingerprint,
                publish_attempt_id,
            ),
            publish_attempt_id,
            file_id,
            group_id: 1,
            sha256,
            runtime_fingerprint,
            media_type: "image/png".to_string(),
            external_id: None,
            vector,
        }
    }

    fn stored_metadata(
        metadata: std::collections::HashMap<String, serde_json::Value>,
    ) -> HashMap<String, qdrant_client::qdrant::Value> {
        metadata
            .into_iter()
            .map(|(key, value)| (key, qdrant_client::qdrant::Value::from(value)))
            .collect()
    }

    #[test]
    fn the_ownership_metadata_binds_shape_collection_and_fingerprint() {
        let metadata = stored_metadata(image_collection_metadata(
            "context69_images_image_v1_a",
            "fingerprint-a",
        ));
        let ownership = image_collection_ownership_from_metadata(&metadata).expect("ownership");
        assert_eq!(ownership.shape_version, IMAGE_COLLECTION_SHAPE_VERSION);
        assert_eq!(ownership.runtime_fingerprint, "fingerprint-a");
        assert_eq!(ownership.collection, "context69_images_image_v1_a");
        assert!(ownership.matches("context69_images_image_v1_a", "fingerprint-a"));
        assert!(!ownership.matches("context69_images_image_v1_b", "fingerprint-a"));
        assert!(!ownership.matches("context69_images_image_v1_a", "fingerprint-b"));

        assert!(
            image_collection_ownership_from_metadata(&HashMap::new()).is_none(),
            "an unmarked pre-existing collection is not owned"
        );
        let mut other_shape = metadata.clone();
        other_shape.insert(
            "context69_image_shape".to_string(),
            qdrant_client::qdrant::Value::from(serde_json::Value::String(
                "other-shape".to_string(),
            )),
        );
        assert!(
            !image_collection_ownership_from_metadata(&other_shape)
                .expect("ownership")
                .matches("context69_images_image_v1_a", "fingerprint-a")
        );
    }

    #[test]
    fn every_image_search_filter_carries_the_group_and_image_record_scope() {
        let unfiltered = format!("{:?}", image_search_filter(7, &[]));
        assert!(unfiltered.contains("group_id"));
        assert!(unfiltered.contains('7'));
        assert!(unfiltered.contains("record_type"));
        assert!(unfiltered.contains("image"));

        let filtered = image_search_filter(7, &["image/jpeg".to_string(), "image/jpg".to_string()]);
        let debug = format!("{filtered:?}");
        assert!(debug.contains("media_type"));
        assert!(debug.contains("image/jpeg"));
        assert!(debug.contains("image/jpg"));
    }

    #[test]
    fn a_dimension_or_non_finite_vector_is_refused_before_the_request() {
        assert!(build_image_points(&[point(vec![0.1, 0.2])], 3).is_err());
        assert!(build_image_points(&[point(vec![0.1, f32::INFINITY])], 2).is_err());
        assert!(build_image_points(&[point(vec![0.1, 0.2])], 2).is_ok());
    }
}
