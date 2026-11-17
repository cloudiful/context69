//! Image vector destination and point identity.
//!
//! Image vectors live in their own physical collection. A point is keyed by the
//! file content revision and the runtime fingerprint, never by a parent
//! document: a replaced or re-embedded image gets a new point, an older point
//! is deleted by exact id, and parent associations are resolved from the
//! current database links at read time. Deleting a parent therefore removes
//! links, not shared vectors.
//!
//! The configured collection name is a namespace for image collections, not an
//! existing collection this channel may adopt. The physical name always adds
//! the image shape and the runtime fingerprint, so an image writer can never
//! mix vectors of another runtime or reuse a document or code collection.

use anyhow::{Result, anyhow};
use uuid::Uuid;

use crate::config::ImageEmbeddingConfig;
use crate::image_embedding::image_embedding_fingerprint;
use crate::qdrant_index::code_vectors::CODE_COLLECTION_PREFIX;

/// The physical collection name of one image runtime and shape.
///
/// The configured collection is only a namespace/prefix, so two deployments
/// that pick the same namespace still get distinct physical collections per
/// fingerprint, and the image shape marker keeps them apart from document and
/// code collections.
pub fn image_collection_name(namespace: &str, runtime_fingerprint: &str) -> String {
    format!("{namespace}_image_v1_{runtime_fingerprint}")
}

/// The revision-aware identity of one image point.
///
/// A changed content revision or a changed runtime produces a new point, so a
/// stale response can never overwrite a newer revision and an older point can
/// be deleted by exact id.
pub fn image_point_id(
    file_id: Uuid,
    sha256: &str,
    runtime_fingerprint: &str,
    publish_attempt_id: Uuid,
) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("context69:image:{file_id}:{sha256}:{runtime_fingerprint}:{publish_attempt_id}")
            .as_bytes(),
    )
}

/// The logical target of one image vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageVectorTarget {
    pub endpoint: String,
    pub collection: String,
    pub runtime_fingerprint: String,
}

impl ImageVectorTarget {
    /// Derive the target from the configured Qdrant endpoint, the image
    /// settings, and the document collection the image channel must not reuse.
    pub fn from_config(
        qdrant_url: &str,
        image: &ImageEmbeddingConfig,
        document_collection: &str,
    ) -> Result<Self> {
        let endpoint = qdrant_url.trim().trim_end_matches('/').to_string();
        let namespace = image.collection_name.trim();
        let document_collection = document_collection.trim();
        if endpoint.is_empty() {
            return Err(anyhow!("the qdrant endpoint must not be empty"));
        }
        if namespace.is_empty() {
            return Err(anyhow!("the image collection namespace must not be empty"));
        }
        if namespace == document_collection {
            return Err(anyhow!(
                "the image collection namespace must not reuse the document collection"
            ));
        }
        if namespace.starts_with(CODE_COLLECTION_PREFIX) {
            return Err(anyhow!(
                "the image collection namespace must not reuse a code collection"
            ));
        }
        let runtime_fingerprint = image_embedding_fingerprint(image);
        let collection = image_collection_name(namespace, &runtime_fingerprint);
        if collection == document_collection || collection.starts_with(CODE_COLLECTION_PREFIX) {
            return Err(anyhow!(
                "the image collection {} aliases a document or code collection",
                collection
            ));
        }
        Ok(Self {
            endpoint,
            collection,
            runtime_fingerprint,
        })
    }
}

/// One image vector point before it becomes a Qdrant `PointStruct`.
#[derive(Debug, Clone)]
pub struct ImageVectorPoint {
    pub point_id: Uuid,
    pub publish_attempt_id: Uuid,
    pub file_id: Uuid,
    pub group_id: i64,
    pub sha256: String,
    pub runtime_fingerprint: String,
    pub media_type: String,
    pub external_id: Option<String>,
    pub vector: Vec<f32>,
}

/// The payload written with one image point.
///
/// Parent links are deliberately absent: the payload is derived from the file
/// content revision and runtime identity only, so relinking or unlinking cannot
/// leave stale parent metadata inside Qdrant.
pub fn image_vector_payload(point: &ImageVectorPoint) -> serde_json::Value {
    serde_json::json!({
        "image_file_id": point.file_id.to_string(),
        "image_point_id": point.point_id.to_string(),
        "publish_attempt_id": point.publish_attempt_id.to_string(),
        "group_id": point.group_id,
        "image_sha256": point.sha256,
        "runtime_fingerprint": point.runtime_fingerprint,
        "record_type": "image",
        "media_type": point.media_type,
        "external_id": point.external_id,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ImageVectorPoint, ImageVectorTarget, image_collection_name, image_point_id,
        image_vector_payload,
    };
    use crate::qdrant_index::code_vectors::code_collection_name;
    use uuid::Uuid;

    fn image_config() -> crate::config::ImageEmbeddingConfig {
        crate::config::ImageEmbeddingConfig {
            base_url: "http://image.invalid/v1".to_string(),
            api_key: None,
            model: "image-model".to_string(),
            query_model: None,
            dimensions: 768,
            timeout: std::time::Duration::from_secs(30),
            collection_name: "context69_images".to_string(),
        }
    }

    fn point(file_id: Uuid, sha256: &str) -> ImageVectorPoint {
        let fingerprint = "runtime-fingerprint";
        let publish_attempt_id = Uuid::new_v4();
        ImageVectorPoint {
            point_id: image_point_id(file_id, sha256, fingerprint, publish_attempt_id),
            publish_attempt_id,
            file_id,
            group_id: 7,
            sha256: sha256.to_string(),
            runtime_fingerprint: fingerprint.to_string(),
            media_type: "image/png".to_string(),
            external_id: Some("cover".to_string()),
            vector: vec![0.1],
        }
    }

    #[test]
    fn the_image_target_is_independent_of_the_document_collection() {
        let target = ImageVectorTarget::from_config(
            "http://qdrant.invalid:6334/",
            &image_config(),
            "context69",
        )
        .expect("target");
        assert_eq!(target.endpoint, "http://qdrant.invalid:6334");
        assert!(!target.collection.is_empty());
        assert!(target.collection.starts_with("context69_images"));
        assert!(target.collection.contains("_image_v1_"));
        assert_ne!(target.collection, "context69");
        assert!(!target.runtime_fingerprint.is_empty());
    }

    #[test]
    fn the_physical_collection_includes_the_shape_and_runtime_fingerprint() {
        let mut other = image_config();
        other.model = "other-model".to_string();
        let first = ImageVectorTarget::from_config(
            "http://qdrant.invalid:6334",
            &image_config(),
            "context69",
        )
        .expect("target");
        let second =
            ImageVectorTarget::from_config("http://qdrant.invalid:6334", &other, "context69")
                .expect("target");
        assert_ne!(first.collection, second.collection);
        assert_eq!(
            first.collection,
            image_collection_name("context69_images", &first.runtime_fingerprint)
        );
        assert!(first.collection.contains("context69_images_image_v1_"));
    }

    #[test]
    fn the_complete_destination_distinguishes_endpoint_collection_and_fingerprint() {
        let target = ImageVectorTarget::from_config(
            "http://qdrant.invalid:6334",
            &image_config(),
            "context69",
        )
        .expect("target");
        let other_endpoint = ImageVectorTarget::from_config(
            "http://other.invalid:6334",
            &image_config(),
            "context69",
        )
        .expect("target");
        let mut other_namespace = image_config();
        other_namespace.collection_name = "other_images".to_string();
        let other_collection = ImageVectorTarget::from_config(
            "http://qdrant.invalid:6334",
            &other_namespace,
            "context69",
        )
        .expect("target");
        let mut other_model = image_config();
        other_model.model = "other-model".to_string();
        let other_fingerprint =
            ImageVectorTarget::from_config("http://qdrant.invalid:6334", &other_model, "context69")
                .expect("target");

        assert_eq!(target, target.clone());
        assert_ne!(target, other_endpoint);
        assert_ne!(target, other_collection);
        assert_ne!(target, other_fingerprint);
    }

    #[test]
    fn the_image_namespace_cannot_alias_document_or_code_collections() {
        let mut document = image_config();
        document.collection_name = "context69".to_string();
        assert!(
            ImageVectorTarget::from_config("http://qdrant.invalid:6334", &document, "context69")
                .is_err()
        );

        let mut code = image_config();
        code.collection_name = code_collection_name("fingerprint");
        assert!(
            ImageVectorTarget::from_config("http://qdrant.invalid:6334", &code, "context69")
                .is_err()
        );
    }

    #[test]
    fn a_point_identity_changes_with_the_content_revision_and_runtime() {
        let file_id = Uuid::new_v4();
        let attempt_a = Uuid::new_v4();
        let attempt_b = Uuid::new_v4();
        assert_ne!(
            image_point_id(file_id, "sha-a", "fingerprint-1", attempt_a),
            image_point_id(file_id, "sha-b", "fingerprint-1", attempt_a)
        );
        assert_ne!(
            image_point_id(file_id, "sha-a", "fingerprint-1", attempt_a),
            image_point_id(file_id, "sha-a", "fingerprint-2", attempt_a)
        );
        assert_ne!(
            image_point_id(file_id, "sha-a", "fingerprint-1", attempt_a),
            image_point_id(file_id, "sha-a", "fingerprint-1", attempt_b)
        );
        assert_eq!(
            image_point_id(file_id, "sha-a", "fingerprint-1", attempt_a),
            image_point_id(file_id, "sha-a", "fingerprint-1", attempt_a)
        );
    }

    #[test]
    fn a_payload_carries_revision_and_runtime_identity_but_no_parent_metadata() {
        let file_id = Uuid::new_v4();
        let point = point(file_id, "abc");
        let payload = image_vector_payload(&point);
        assert_eq!(payload["image_file_id"], file_id.to_string());
        assert_eq!(payload["image_point_id"], point.point_id.to_string());
        assert_eq!(
            payload["publish_attempt_id"],
            point.publish_attempt_id.to_string()
        );
        assert_eq!(payload["record_type"], "image");
        assert_eq!(payload["image_sha256"], "abc");
        assert_eq!(payload["runtime_fingerprint"], "runtime-fingerprint");
        assert_eq!(payload["group_id"], 7);
        assert!(payload.get("document_id").is_none());
        assert!(payload.get("parents").is_none());
    }

    #[test]
    fn an_empty_namespace_is_refused() {
        let mut config = image_config();
        config.collection_name = "   ".to_string();
        assert!(
            ImageVectorTarget::from_config("http://qdrant.invalid:6334", &config, "context69")
                .is_err()
        );
    }
}
