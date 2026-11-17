//! Scoped image search, document image lists, and occurrence unlinking.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use uuid::Uuid;

use super::LibraryService;
use super::image_links::ImageOccurrenceRole;
use super::ingest_types::LibraryImageKind;
use crate::config::QdrantConfig;
use crate::contracts::{
    DocumentImageListResponse, DocumentImageOccurrence, DocumentImageOccurrenceWithParent,
    IMAGE_DOCUMENT_LIST_MAX, ImageSearchHit, ImageSearchRequest, ImageSearchResponse,
    ImageUnlinkOutcome, UnlinkDocumentImageResult, UnlinkDocumentImagesRequest,
    UnlinkDocumentImagesResponse,
};
use crate::domain::{AccessScope, GroupRecord};
use crate::domain_errors::DomainError;
use crate::library_store::{
    IMAGE_PARENT_HYDRATION_LIMIT, ImageParentDocumentRecord, LibraryImageFileRecord,
};
use crate::qdrant_index::QdrantIndex;

/// Candidate oversampling before stale points are dropped.
const IMAGE_SEARCH_CANDIDATE_MULTIPLIER: usize = 3;

/// The largest candidate window one search asks Qdrant for.
const IMAGE_SEARCH_CANDIDATE_MAX: usize = 200;

/// The number of runtime snapshots one search tries before it reports a
/// changing configuration instead of waiting.
const IMAGE_SEARCH_SNAPSHOT_ATTEMPTS: usize = 2;

fn candidate_limit(limit: usize) -> u64 {
    limit
        .saturating_mul(IMAGE_SEARCH_CANDIDATE_MULTIPLIER)
        .min(IMAGE_SEARCH_CANDIDATE_MAX)
        .max(limit) as u64
}

fn media_types(value: Option<&str>) -> Result<Vec<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let kind = LibraryImageKind::from_media_type(value).ok_or_else(|| {
        DomainError::invalid_argument(
            "image search media_type must be image/png, image/jpeg, image/jpg, or image/webp",
        )
    })?;
    Ok(match kind {
        LibraryImageKind::Png => vec!["image/png".to_string()],
        LibraryImageKind::Jpeg => vec!["image/jpeg".to_string(), "image/jpg".to_string()],
        LibraryImageKind::WebP => vec!["image/webp".to_string()],
    })
}

fn parents_for_file(
    links: &[crate::library_store::ImageDocumentLinkRecord],
    documents: &HashMap<i64, ImageParentDocumentRecord>,
) -> (Vec<DocumentImageOccurrenceWithParent>, bool) {
    let has_more = links.len() > IMAGE_PARENT_HYDRATION_LIMIT;
    let parents = links
        .iter()
        .take(IMAGE_PARENT_HYDRATION_LIMIT)
        .filter_map(|link| {
            let document = documents.get(&link.document_id)?;
            let role = link.role.parse::<ImageOccurrenceRole>().ok()?;
            Some(DocumentImageOccurrenceWithParent {
                document_id: link.document_id,
                document_key: crate::contracts::DocumentKey {
                    source_key: document.source_key.clone(),
                    external_id: document.external_id.clone(),
                },
                title: document.title.clone(),
                document_source_uri: document.source_uri.clone(),
                role,
                ordinal: link.ordinal,
                caption: link.caption.clone(),
                context: link.context_text.clone(),
                source_uri: link.source_uri.clone(),
            })
        })
        .collect();
    (parents, has_more)
}

fn occurrence(
    link: &crate::library_store::ImageDocumentLinkRecord,
    file: &LibraryImageFileRecord,
) -> Option<DocumentImageOccurrence> {
    Some(DocumentImageOccurrence {
        file_id: file.file_id,
        external_id: file.external_id.clone(),
        filename: file.filename.clone(),
        media_type: file.media_type.clone(),
        sha256: file.sha256.clone(),
        role: link.role.parse().ok()?,
        ordinal: link.ordinal,
        caption: link.caption.clone(),
        context: link.context_text.clone(),
        source_uri: link.source_uri.clone(),
    })
}

impl LibraryService {
    /// Search the independent image vector space inside one authorized group.
    ///
    /// The query is embedded without holding the settings publication guard.
    /// The guard is taken only for the bounded read admission, where the
    /// snapshot is re-read and must still be the exact one that embedded the
    /// query; a changed configuration is retried instead of blocking settings
    /// across provider latency.
    pub async fn search_images(
        &self,
        group: &GroupRecord,
        scope: &AccessScope,
        request: ImageSearchRequest,
    ) -> Result<ImageSearchResponse> {
        request
            .validate()
            .map_err(|message| anyhow::Error::from(DomainError::invalid_argument(message)))?;
        let query = request.query.trim().to_string();
        let media_types = media_types(request.media_type.as_deref())?;
        let candidate_limit = candidate_limit(request.limit);

        let hits = self
            .search_image_candidates(
                group,
                scope,
                &query,
                &media_types,
                candidate_limit,
                request.limit,
            )
            .await?;
        Ok(ImageSearchResponse { items: hits })
    }

    async fn search_image_candidates(
        &self,
        group: &GroupRecord,
        scope: &AccessScope,
        query: &str,
        media_types: &[String],
        candidate_limit: u64,
        limit: usize,
    ) -> Result<Vec<ImageSearchHit>> {
        for attempt in 0..IMAGE_SEARCH_SNAPSHOT_ATTEMPTS {
            let snapshot = self
                .settings
                .image_embedding_snapshot()
                .ok_or_else(|| DomainError::unavailable("image embedding is not configured"))?;
            let destination = snapshot.destination.clone().ok_or_else(|| {
                DomainError::unavailable("image search has no configured vector destination")
            })?;
            let vector = snapshot
                .provider
                .embed_query(query)
                .await
                .map_err(|error| {
                    DomainError::upstream_error(format!("image query embedding failed: {error}"))
                })?;
            if vector.len() != snapshot.config.dimensions
                || !vector.iter().all(|value| value.is_finite())
            {
                return Err(DomainError::upstream_error(
                    "image query embedding returned an invalid vector",
                )
                .into());
            }

            let _guard = self.settings.image_embedding_publication_read().await;
            let current = self
                .settings
                .image_embedding_snapshot()
                .ok_or_else(|| DomainError::unavailable("image embedding was disabled"))?;
            if !Arc::ptr_eq(&snapshot, &current) {
                drop(_guard);
                if attempt + 1 == IMAGE_SEARCH_SNAPSHOT_ATTEMPTS {
                    return Err(DomainError::unavailable(
                        "image search configuration changed during the request; retry",
                    )
                    .into());
                }
                continue;
            }

            let index = QdrantIndex::attach_image_read_only_collection(
                &QdrantConfig {
                    url: destination.endpoint.clone(),
                    collection_name: destination.collection.clone(),
                    recreate_on_dimension_mismatch: false,
                },
                &destination.collection,
                snapshot.config.dimensions,
                &destination.runtime_fingerprint,
            )
            .await
            .map_err(|error| {
                DomainError::unavailable(format!("image vector index unavailable: {error}"))
            })?;
            let points = index
                .search_image_points(group.id, media_types, vector, candidate_limit)
                .await
                .map_err(|error| {
                    DomainError::unavailable(format!("image vector search failed: {error}"))
                })?;
            drop(_guard);

            let point_ids: Vec<Uuid> = points.iter().map(|point| point.chunk_id).collect();
            let candidates = self
                .db
                .active_image_search_candidates(
                    group.id,
                    &destination.endpoint,
                    &destination.collection,
                    &destination.runtime_fingerprint,
                    &point_ids,
                )
                .await?;
            let mut by_point: HashMap<Uuid, crate::db::ActiveImageSearchCandidate> = candidates
                .into_iter()
                .map(|candidate| (candidate.point_id, candidate))
                .collect();

            let mut selected: Vec<(f32, crate::db::ActiveImageSearchCandidate)> = Vec::new();
            for point in points {
                let Some(candidate) = by_point.remove(&point.chunk_id) else {
                    continue;
                };
                selected.push((point.score, candidate));
                if selected.len() == limit {
                    break;
                }
            }

            let file_ids: Vec<Uuid> = selected
                .iter()
                .map(|(_, candidate)| candidate.file_id)
                .collect();
            let links = self
                .store
                .image_links_for_files_bounded(group.id, &file_ids, IMAGE_PARENT_HYDRATION_LIMIT)
                .await?;
            let mut links_by_file: HashMap<Uuid, Vec<_>> = HashMap::new();
            for link in links {
                links_by_file.entry(link.file_id).or_default().push(link);
            }
            let document_ids: Vec<i64> = links_by_file
                .values()
                .flatten()
                .map(|link| link.document_id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let documents = self
                .store
                .authorized_image_parent_documents(group.id, &document_ids, scope)
                .await?;
            let documents: HashMap<i64, ImageParentDocumentRecord> = documents
                .into_iter()
                .map(|document| (document.document_id, document))
                .collect();

            let hits = selected
                .into_iter()
                .map(|(score, candidate)| {
                    let empty = Vec::new();
                    let (parents, has_more_parents) = parents_for_file(
                        links_by_file.get(&candidate.file_id).unwrap_or(&empty),
                        &documents,
                    );
                    ImageSearchHit {
                        score,
                        file_id: candidate.file_id,
                        external_id: candidate.external_id,
                        filename: candidate.filename,
                        media_type: candidate.media_type,
                        sha256: candidate.sha256,
                        parents,
                        has_more_parents,
                    }
                })
                .collect();
            return Ok(hits);
        }
        Err(DomainError::unavailable(
            "image search configuration changed during the request; retry",
        )
        .into())
    }

    /// The bounded current image occurrences of one authorized parent document.
    pub async fn list_document_images(
        &self,
        group_id: i64,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<DocumentImageListResponse> {
        self.db
            .get_document(document_id, scope)
            .await?
            .ok_or_else(|| DomainError::not_found("document not found"))?;
        let links = self
            .store
            .image_links_for_document_bounded(group_id, document_id, IMAGE_DOCUMENT_LIST_MAX)
            .await?;
        let has_more = links.len() > IMAGE_DOCUMENT_LIST_MAX;
        let links = links
            .into_iter()
            .take(IMAGE_DOCUMENT_LIST_MAX)
            .collect::<Vec<_>>();
        let file_ids: Vec<Uuid> = links.iter().map(|link| link.file_id).collect();
        let files: HashMap<Uuid, LibraryImageFileRecord> = self
            .store
            .images_by_ids_in_group(group_id, &file_ids)
            .await?
            .into_iter()
            .map(|file| (file.file_id, file))
            .collect();
        let items = links
            .iter()
            .filter_map(|link| {
                files
                    .get(&link.file_id)
                    .and_then(|file| occurrence(link, file))
            })
            .collect();
        Ok(DocumentImageListResponse {
            document_id,
            items,
            has_more,
        })
    }

    /// Remove bounded occurrence links without touching files or vectors.
    pub async fn unlink_document_images(
        &self,
        group_id: i64,
        request: UnlinkDocumentImagesRequest,
    ) -> Result<UnlinkDocumentImagesResponse> {
        request
            .validate()
            .map_err(|message| anyhow::Error::from(DomainError::invalid_argument(message)))?;
        let mut items = Vec::with_capacity(request.items.len());
        for item in request.items {
            let deleted = self
                .store
                .delete_image_document_link(
                    group_id,
                    item.file_id,
                    item.document_id,
                    item.role.as_str(),
                    item.ordinal,
                )
                .await?;
            items.push(UnlinkDocumentImageResult {
                file_id: item.file_id,
                document_id: item.document_id,
                role: item.role,
                ordinal: item.ordinal,
                outcome: if deleted {
                    ImageUnlinkOutcome::Unlinked
                } else {
                    ImageUnlinkOutcome::NotLinked
                },
            });
        }
        Ok(UnlinkDocumentImagesResponse { items })
    }
}

#[cfg(test)]
mod tests {
    use super::{candidate_limit, media_types, parents_for_file};
    use crate::library_store::{IMAGE_PARENT_HYDRATION_LIMIT, ImageDocumentLinkRecord};

    fn link(index: usize) -> ImageDocumentLinkRecord {
        ImageDocumentLinkRecord {
            file_id: uuid::Uuid::nil(),
            document_id: index as i64,
            group_id: 1,
            role: "body".to_string(),
            ordinal: index as i32,
            caption: None,
            context_text: None,
            source_uri: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn candidate_oversampling_is_bounded_and_never_below_the_requested_limit() {
        assert_eq!(candidate_limit(1), 3);
        assert_eq!(candidate_limit(20), 60);
        assert_eq!(candidate_limit(50), 150);
    }

    #[test]
    fn only_supported_media_types_reach_the_filter() {
        assert_eq!(media_types(None).unwrap(), Vec::<String>::new());
        assert_eq!(
            media_types(Some("image/jpg")).unwrap(),
            vec!["image/jpeg".to_string(), "image/jpg".to_string()]
        );
        assert!(media_types(Some("image/gif")).is_err());
    }

    #[test]
    fn canonical_kind_values_are_the_published_filter_values() {
        use crate::services::library::LibraryImageKind;
        for kind in [
            LibraryImageKind::Png,
            LibraryImageKind::Jpeg,
            LibraryImageKind::WebP,
        ] {
            let filter = media_types(Some(kind.media_type())).unwrap();
            assert!(
                filter.contains(&kind.media_type().to_string()),
                "stored canonical {kind:?} must match the published filter {filter:?}"
            );
        }
        assert_eq!(
            media_types(Some("IMAGE/PNG")).unwrap(),
            vec!["image/png".to_string()]
        );
    }

    #[test]
    fn parent_hydration_is_bounded_and_reports_more() {
        let links = (0..IMAGE_PARENT_HYDRATION_LIMIT + 1)
            .map(link)
            .collect::<Vec<_>>();
        let (parents, has_more) = parents_for_file(&links, &Default::default());
        assert!(parents.is_empty(), "no authorized parent is hydrated");
        assert!(has_more);

        let (parents, has_more) = parents_for_file(&links[..1], &Default::default());
        assert!(parents.is_empty());
        assert!(!has_more);
    }
}
