//! Parent/occurrence links of a stored image.
//!
//! A link names the parent document by its group-scoped `DocumentKey`, so the
//! parent is resolved inside the caller's group and a link can never reach a
//! document of another group. The image file itself is never deleted by a link
//! removal: a shared image keeps serving every other parent.

use anyhow::Result;
use uuid::Uuid;

use super::LibraryService;
use super::image_validation;
use crate::contracts::DocumentKey;
use crate::domain_errors::DomainError;
use crate::library_store::{ImageDocumentLinkRecord, NewImageDocumentLink};

pub use context69_contracts::ImageOccurrenceRole;

/// The longest accepted caption of one occurrence.
pub const MAX_IMAGE_CAPTION_CHARS: usize = 1024;

/// The longest accepted surrounding text of one occurrence.
pub const MAX_IMAGE_CONTEXT_CHARS: usize = 8192;

/// The longest accepted source URL of one occurrence.
pub const MAX_IMAGE_SOURCE_URI_CHARS: usize = 2048;

/// One image-to-parent link request.
#[derive(Debug, Clone)]
pub struct ImageLinkRequest<'a> {
    /// The parent document identity, resolved only inside the caller's group.
    pub parent: &'a DocumentKey,
    pub role: ImageOccurrenceRole,
    pub ordinal: i32,
    pub caption: Option<&'a str>,
    pub context: Option<&'a str>,
    pub source_uri: Option<&'a str>,
}

#[derive(Debug)]
struct ValidatedImageLink {
    source_key: String,
    external_id: String,
    role: ImageOccurrenceRole,
    ordinal: i32,
    caption: Option<String>,
    context_text: Option<String>,
    source_uri: Option<String>,
}

fn bounded_text(value: Option<&str>, field: &str, max_chars: usize) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let chars = value.chars().count();
    if chars > max_chars {
        return Err(DomainError::invalid_argument(format!(
            "the image occurrence {field} is {chars} characters; the limit is {max_chars}"
        ))
        .into());
    }
    Ok(Some(value.to_string()))
}

fn validate_parent_key(parent: &DocumentKey) -> Result<(String, String)> {
    let source_key = parent.source_key.trim();
    let external_id = parent.external_id.trim();
    if source_key.is_empty() || external_id.is_empty() {
        return Err(DomainError::invalid_argument(
            "the parent document key must name a source and an id",
        )
        .into());
    }
    Ok((source_key.to_string(), external_id.to_string()))
}

fn validate_image_link(request: &ImageLinkRequest<'_>) -> Result<ValidatedImageLink> {
    let (source_key, external_id) = validate_parent_key(request.parent)?;
    if request.ordinal < 0 {
        return Err(DomainError::invalid_argument(
            "the image occurrence ordinal must not be negative",
        )
        .into());
    }
    let source_uri = bounded_text(request.source_uri, "source URL", MAX_IMAGE_SOURCE_URI_CHARS)?;
    if let Some(source_uri) = source_uri.as_deref()
        && !(source_uri.starts_with("http://") || source_uri.starts_with("https://"))
    {
        return Err(DomainError::invalid_argument(
            "the image occurrence source URL must be an http or https URL",
        )
        .into());
    }
    Ok(ValidatedImageLink {
        source_key,
        external_id,
        role: request.role,
        ordinal: request.ordinal,
        caption: bounded_text(request.caption, "caption", MAX_IMAGE_CAPTION_CHARS)?,
        context_text: bounded_text(request.context, "context", MAX_IMAGE_CONTEXT_CHARS)?,
        source_uri,
    })
}

impl LibraryService {
    /// Attach one stored image to a parent document occurrence.
    ///
    /// The parent is resolved by its group-scoped key, so a missing parent is a
    /// not-found failure and no placeholder document is ever created. The file
    /// must already be a supported image in the same group; the occurrence slot
    /// belongs to one image, and an identical repeat converges on the same row.
    pub async fn link_image_to_document(
        &self,
        group_id: i64,
        file_id: Uuid,
        request: &ImageLinkRequest<'_>,
    ) -> Result<ImageDocumentLinkRecord> {
        let link = validate_image_link(request)?;
        let mut tx = self.db.pool().begin().await?;
        // The shared group/file scopes are taken before the entity rechecks, so
        // a file or parent delete of this group either finishes first (and the
        // recheck fails) or waits for this link to commit and then removes it.
        self.db
            .lock_image_vector_scopes_in(&mut tx, group_id, &[file_id], &[])
            .await?;
        let media = self
            .store
            .image_file_media_in_group_on(&mut tx, group_id, file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        image_validation::require_image_media(&media.0, &media.1)?;
        let document_id = self
            .store
            .image_document_id_by_key_on(&mut tx, group_id, &link.source_key, &link.external_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found("the parent document was not found in this group")
            })?;
        self.db
            .lock_image_vector_scopes_in(&mut tx, group_id, &[], &[document_id])
            .await?;
        if let Some(existing_file) = self
            .store
            .image_occurrence_file_id_on(
                &mut tx,
                group_id,
                document_id,
                link.role.as_str(),
                link.ordinal,
            )
            .await?
            && existing_file != file_id
        {
            return Err(DomainError::conflict(
                "the document occurrence already belongs to another image",
            )
            .into());
        }
        self.store
            .upsert_image_document_link_on(
                &mut tx,
                &NewImageDocumentLink {
                    file_id,
                    document_id,
                    group_id,
                    role: link.role.as_str(),
                    ordinal: link.ordinal,
                    caption: link.caption.as_deref(),
                    context_text: link.context_text.as_deref(),
                    source_uri: link.source_uri.as_deref(),
                },
            )
            .await?;
        tx.commit().await?;
        self.store
            .find_image_document_link(
                group_id,
                file_id,
                document_id,
                link.role.as_str(),
                link.ordinal,
            )
            .await?
            .ok_or_else(|| DomainError::not_found("image document link not found").into())
    }

    /// Remove one scoped occurrence link without touching the stored image.
    pub async fn unlink_image_from_document(
        &self,
        group_id: i64,
        file_id: Uuid,
        parent: &DocumentKey,
        role: ImageOccurrenceRole,
        ordinal: i32,
    ) -> Result<bool> {
        let (source_key, external_id) = validate_parent_key(parent)?;
        if ordinal < 0 {
            return Err(DomainError::invalid_argument(
                "the image occurrence ordinal must not be negative",
            )
            .into());
        }
        let mut tx = self.db.pool().begin().await?;
        self.db
            .lock_image_vector_scopes_in(&mut tx, group_id, &[file_id], &[])
            .await?;
        if self
            .store
            .image_file_media_in_group_on(&mut tx, group_id, file_id)
            .await?
            .is_none()
        {
            return Err(DomainError::not_found(format!("unknown file {file_id}")).into());
        }
        let document_id = self
            .store
            .image_document_id_by_key_on(&mut tx, group_id, &source_key, &external_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found("the parent document was not found in this group")
            })?;
        self.db
            .lock_image_vector_scopes_in(&mut tx, group_id, &[], &[document_id])
            .await?;
        let deleted = self
            .store
            .delete_image_document_link_on(
                &mut tx,
                group_id,
                file_id,
                document_id,
                role.as_str(),
                ordinal,
            )
            .await?;
        tx.commit().await?;
        Ok(deleted)
    }

    /// The bounded occurrence links of one stored image.
    pub async fn list_file_image_links(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<Vec<ImageDocumentLinkRecord>> {
        self.store
            .list_image_document_links_for_file(group_id, file_id)
            .await
    }

    /// The bounded occurrence links of one parent document.
    pub async fn list_document_image_links(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<Vec<ImageDocumentLinkRecord>> {
        self.store
            .list_image_document_links_for_document(group_id, document_id)
            .await
    }

    /// The explicit file cleanup of a delete: every scoped link of one file.
    pub async fn delete_file_image_links(&self, group_id: i64, file_id: Uuid) -> Result<u64> {
        self.store
            .delete_image_document_links_for_file(group_id, file_id)
            .await
    }

    /// The explicit parent cleanup of a delete: every scoped link of one
    /// document.
    pub async fn delete_document_image_links(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<u64> {
        self.store
            .delete_image_document_links_for_document(group_id, document_id)
            .await
    }

    /// The explicit parent cleanup of a bounded document set.
    pub async fn delete_documents_image_links(
        &self,
        group_id: i64,
        document_ids: &[i64],
    ) -> Result<u64> {
        self.store
            .delete_image_document_links_for_documents(group_id, document_ids)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ImageLinkRequest, ImageOccurrenceRole, MAX_IMAGE_CAPTION_CHARS, MAX_IMAGE_CONTEXT_CHARS,
        validate_image_link,
    };
    use crate::contracts::DocumentKey;
    use crate::domain_errors::{DomainError, find_domain_error};

    fn request<'a>(parent: &'a DocumentKey) -> ImageLinkRequest<'a> {
        ImageLinkRequest {
            parent,
            role: ImageOccurrenceRole::Cover,
            ordinal: 0,
            caption: None,
            context: None,
            source_uri: None,
        }
    }

    #[test]
    fn occurrence_roles_parse_cover_and_body_only() {
        assert_eq!(
            " cover ".parse::<ImageOccurrenceRole>().unwrap(),
            ImageOccurrenceRole::Cover
        );
        assert_eq!(
            "body".parse::<ImageOccurrenceRole>().unwrap(),
            ImageOccurrenceRole::Body
        );
        assert_eq!(ImageOccurrenceRole::Cover.as_str(), "cover");
        let error = "thumbnail"
            .parse::<ImageOccurrenceRole>()
            .expect_err("unknown role");
        assert_eq!(
            error,
            "unknown image occurrence role thumbnail; expected cover or body"
        );
    }

    #[test]
    fn a_link_requires_a_complete_parent_key_and_a_non_negative_ordinal() {
        let empty = DocumentKey {
            source_key: " ".into(),
            external_id: "doc-1".into(),
        };
        assert!(validate_image_link(&request(&empty)).is_err());

        let parent = DocumentKey {
            source_key: "file_library".into(),
            external_id: "doc-1".into(),
        };
        let mut invalid_ordinal = request(&parent);
        invalid_ordinal.ordinal = -1;
        let error = validate_image_link(&invalid_ordinal).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the image occurrence ordinal must not be negative"
        );
    }

    #[test]
    fn bounded_text_is_trimmed_dropped_when_empty_and_limited() {
        let parent = DocumentKey {
            source_key: "file_library".into(),
            external_id: "doc-1".into(),
        };
        let mut link = request(&parent);
        link.caption = Some("  Figure 1  ");
        link.context = Some("   ");
        let validated = validate_image_link(&link).unwrap();
        assert_eq!(validated.caption.as_deref(), Some("Figure 1"));
        assert_eq!(validated.context_text, None);

        let mut too_long = request(&parent);
        let long_caption = "c".repeat(MAX_IMAGE_CAPTION_CHARS + 1);
        too_long.caption = Some(long_caption.as_str());
        let error = validate_image_link(&too_long).unwrap_err();
        assert!(
            matches!(
                find_domain_error(&error),
                Some(DomainError::InvalidArgument(_))
            ),
            "{error}"
        );

        let mut too_long_context = request(&parent);
        let long_context = "c".repeat(MAX_IMAGE_CONTEXT_CHARS + 1);
        too_long_context.context = Some(long_context.as_str());
        assert!(validate_image_link(&too_long_context).is_err());
    }

    #[test]
    fn source_urls_must_be_absolute_http_urls() {
        let parent = DocumentKey {
            source_key: "file_library".into(),
            external_id: "doc-1".into(),
        };
        let mut link = request(&parent);
        link.source_uri = Some("https://example.test/figure.png");
        assert_eq!(
            validate_image_link(&link).unwrap().source_uri.as_deref(),
            Some("https://example.test/figure.png")
        );
        link.source_uri = Some("file:///etc/passwd");
        let error = validate_image_link(&link).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the image occurrence source URL must be an http or https URL"
        );
    }

    /// A repeat occurrence of the same image may use a distinct role/ordinal,
    /// so the request validation keeps the caller's ordinal rather than
    /// rewriting it.
    #[test]
    fn distinct_occurrences_keep_their_role_and_ordinal() {
        let parent = DocumentKey {
            source_key: "file_library".into(),
            external_id: "doc-1".into(),
        };
        for (role, ordinal) in [
            (ImageOccurrenceRole::Cover, 0),
            (ImageOccurrenceRole::Body, 0),
            (ImageOccurrenceRole::Body, 2),
        ] {
            let mut link = request(&parent);
            link.role = role;
            link.ordinal = ordinal;
            let validated = validate_image_link(&link).unwrap();
            assert_eq!(validated.role, role);
            assert_eq!(validated.ordinal, ordinal);
        }
    }
}
