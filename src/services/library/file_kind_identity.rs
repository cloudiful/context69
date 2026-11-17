//! The stored file kind that one group external id may keep across a
//! replacement.
//!
//! An image and a non-image file are different entities even when they share a
//! group external id. Replacing one with the other would leave image occurrence
//! links attached to a text file, or retain an image vector for content that is
//! no longer an image, so the replacement is refused and the caller must delete
//! the old file or choose a different external id.

use anyhow::Result;

use crate::domain_errors::DomainError;

use super::image_validation::{declared_image_kind, image_kind_from_bytes};
use super::storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredFileKind {
    Image,
    Document,
}

impl StoredFileKind {
    fn describe(self) -> &'static str {
        match self {
            Self::Image => "an image file",
            Self::Document => "a non-image file",
        }
    }
}

/// Refuses options that only make sense for a text/document item.
///
/// Image content has no translation, chunking, or text-conversion stage, so an
/// image request that carries a translation directive is rejected before any
/// storage or replacement work instead of silently running a post-commit text
/// flow.
pub(crate) fn ensure_image_options_supported(
    options: &crate::contracts::IngestOptions,
) -> Result<()> {
    if options.translation.is_some() {
        return Err(DomainError::invalid_argument(
            "image files do not support translation options",
        )
        .into());
    }
    if options.source_policy.is_release() {
        return Err(DomainError::invalid_argument(
            "image files retain their original source; release_after_processing is not supported",
        )
        .into());
    }
    Ok(())
}

/// The kind an upload or stored row belongs to.
///
/// Upload bytes win when they are available: the image magic, not the declared
/// media type alone, decides whether they are an image. A stored row without
/// bytes is identified by its validated filename/media type and document
/// classification.
pub(crate) fn stored_file_kind(
    filename: &str,
    media_type: &str,
    bytes: Option<&[u8]>,
) -> Result<StoredFileKind> {
    if bytes.is_some_and(|bytes| image_kind_from_bytes(bytes).is_some())
        || declared_image_kind(filename, media_type)?.is_some()
    {
        return Ok(StoredFileKind::Image);
    }
    storage::detect_file_kind(filename, media_type)?;
    Ok(StoredFileKind::Document)
}

/// Refuses a replacement that would change an external id from image to
/// non-image or the reverse.
pub(crate) fn ensure_external_id_replacement_kind(
    existing_filename: &str,
    existing_media_type: &str,
    new_filename: &str,
    new_media_type: &str,
    new_bytes: Option<&[u8]>,
) -> Result<()> {
    let existing = stored_file_kind(existing_filename, existing_media_type, None)?;
    let new = stored_file_kind(new_filename, new_media_type, new_bytes)?;
    if existing != new {
        return Err(DomainError::conflict(format!(
            "cannot replace {} with {} through the same external_id; delete the old file or choose \
             a different external_id",
            existing.describe(),
            new.describe()
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        StoredFileKind, ensure_external_id_replacement_kind, ensure_image_options_supported,
        stored_file_kind,
    };
    use crate::services::library::image_validation::PNG_MAGIC;

    fn png_bytes() -> Vec<u8> {
        // The magic prefix is what the kind classifier reads; no decoder runs.
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&[0; 32]);
        bytes
    }

    #[test]
    fn bytes_decide_image_identity_before_the_declared_media_type() {
        let bytes = png_bytes();
        assert_eq!(
            stored_file_kind("cover.bin", "application/octet-stream", Some(&bytes)).expect("kind"),
            StoredFileKind::Image
        );
        assert_eq!(
            stored_file_kind("notes.md", "text/markdown", Some(b"hello".as_slice())).expect("kind"),
            StoredFileKind::Document
        );
    }

    #[test]
    fn an_image_request_with_a_translation_directive_is_refused() {
        let mut options = crate::contracts::IngestOptions::retain();
        assert!(ensure_image_options_supported(&options).is_ok());
        options.translation = Some(crate::contracts::TranslationDirective {
            source_locale: None,
            target_locales: vec!["de".to_string()],
        });
        assert!(ensure_image_options_supported(&options).is_err());

        let mut release = crate::contracts::IngestOptions::retain();
        release.source_policy = crate::contracts::SourcePolicy::ReleaseAfterProcessing;
        assert!(ensure_image_options_supported(&release).is_err());
    }

    #[test]
    fn sha_reuse_checks_the_existing_row_kind_before_returning_it() {
        let bytes = png_bytes();
        assert!(
            ensure_external_id_replacement_kind(
                "notes.txt",
                "text/plain",
                "cover.png",
                "image/png",
                Some(&bytes),
            )
            .is_err(),
            "an image upload must not reuse a document row by SHA"
        );
    }

    #[test]
    fn a_cross_kind_external_id_replacement_is_refused() {
        assert!(
            ensure_external_id_replacement_kind(
                "notes.md",
                "text/markdown",
                "cover.png",
                "image/png",
                Some(&png_bytes()),
            )
            .is_err()
        );
        assert!(
            ensure_external_id_replacement_kind(
                "cover.png",
                "image/png",
                "notes.md",
                "text/markdown",
                Some(b"hello".as_slice()),
            )
            .is_err()
        );
        assert!(
            ensure_external_id_replacement_kind(
                "cover.png",
                "image/png",
                "other.png",
                "image/png",
                Some(&png_bytes()),
            )
            .is_ok()
        );
        assert!(
            ensure_external_id_replacement_kind(
                "notes.md",
                "text/markdown",
                "other.md",
                "text/markdown",
                Some(b"hello".as_slice()),
            )
            .is_ok()
        );
    }
}
