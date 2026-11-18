use sha2::{Digest, Sha256};

use crate::contracts::LibraryTextContentFormat;

use super::*;

/// The classification of one upload: an existing document kind, or the
/// additive supported image kind.
///
/// An image claim is decided before document detection, so a declared image
/// never falls through to the document kinds. An upload that does not claim to
/// be an image is classified exactly as before by [`detect_file_kind`].
pub(crate) fn detect_media_kind(
    filename: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<LibraryMediaKind> {
    let Some(declared) = image_validation::declared_image_kind(filename, media_type)? else {
        return Ok(LibraryMediaKind::Document(detect_file_kind(
            filename, media_type,
        )?));
    };
    let detected = image_validation::image_kind_from_bytes(bytes).ok_or_else(|| {
        DomainError::invalid_argument(format!(
            "image {filename} does not contain PNG, JPEG or WebP bytes"
        ))
    })?;
    if detected != declared {
        return Err(DomainError::invalid_argument(format!(
            "image {filename} declares {} but contains {} bytes",
            declared.media_type(),
            detected.media_type()
        ))
        .into());
    }
    Ok(LibraryMediaKind::Image(detected))
}

pub(crate) fn detect_file_kind(filename: &str, media_type: &str) -> Result<LibraryFileKind> {
    let lowered = filename.to_ascii_lowercase();
    if lowered.ends_with(".pdf") || media_type == "application/pdf" {
        return Ok(LibraryFileKind::Pdf);
    }
    if lowered.ends_with(".docx")
        || media_type == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    {
        return Ok(LibraryFileKind::Docx);
    }
    if lowered.ends_with(".xlsx")
        || media_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    {
        return Ok(LibraryFileKind::Xlsx);
    }
    if lowered.ends_with(".txt")
        || lowered.ends_with(".md")
        || lowered.ends_with(".json")
        || media_type.starts_with("text/")
        || media_type == "application/json"
    {
        return Ok(LibraryFileKind::PlainText);
    }
    Err(DomainError::invalid_argument(format!("unsupported file type for {filename}")).into())
}

pub(super) fn text_filename_from_title(title: &str, format: LibraryTextContentFormat) -> String {
    let mut stem = title
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other if other.is_control() => ' ',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_string();
    if stem.is_empty() {
        stem = "text-entry".to_string();
    }
    let desired_extension = match format {
        LibraryTextContentFormat::PlainText => ".txt",
        LibraryTextContentFormat::Markdown => ".md",
    };
    let lowered = stem.to_ascii_lowercase();
    if lowered.ends_with(".txt") {
        stem.truncate(stem.len() - 4);
    } else if lowered.ends_with(".md") {
        stem.truncate(stem.len() - 3);
    }
    if !stem.to_ascii_lowercase().ends_with(desired_extension) {
        stem.push_str(desired_extension);
    }
    stem
}

pub(super) fn text_media_type(format: LibraryTextContentFormat) -> &'static str {
    match format {
        LibraryTextContentFormat::PlainText => "text/plain",
        LibraryTextContentFormat::Markdown => "text/markdown",
    }
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::contracts::LibraryTextContentFormat;

    use super::{detect_file_kind, detect_media_kind, text_filename_from_title, text_media_type};
    use crate::services::library::{LibraryFileKind, LibraryImageKind, LibraryMediaKind};

    #[test]
    fn unknown_file_type_is_typed_invalid_argument() {
        use crate::domain_errors::{DomainError, find_domain_error};

        let error = detect_file_kind("payload.exe", "application/octet-stream").unwrap_err();
        assert!(matches!(
            find_domain_error(&error),
            Some(DomainError::InvalidArgument(_))
        ));
        assert_eq!(error.to_string(), "unsupported file type for payload.exe");
    }

    #[test]
    fn image_bytes_classify_as_the_declared_supported_image_kind() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(
            detect_media_kind("cover.png", "image/png", png).unwrap(),
            LibraryMediaKind::Image(LibraryImageKind::Png)
        );
        assert_eq!(
            detect_media_kind("cover.jpg", "image/jpeg", b"\xff\xd8\xff").unwrap(),
            LibraryMediaKind::Image(LibraryImageKind::Jpeg)
        );
        assert_eq!(
            detect_media_kind("cover.webp", "image/webp", b"RIFF\0\0\0\0WEBP").unwrap(),
            LibraryMediaKind::Image(LibraryImageKind::WebP)
        );
    }

    #[test]
    fn nonimage_uploads_keep_the_existing_document_classification() {
        assert_eq!(
            detect_media_kind("notes.md", "text/markdown", b"# notes").unwrap(),
            LibraryMediaKind::Document(LibraryFileKind::PlainText)
        );
        assert_eq!(
            detect_media_kind("report.pdf", "application/pdf", b"%PDF").unwrap(),
            LibraryMediaKind::Document(LibraryFileKind::Pdf)
        );
    }

    #[test]
    fn image_claims_with_foreign_or_refused_bytes_fail_explicitly() {
        use crate::domain_errors::{DomainError, find_domain_error};

        let error = detect_media_kind("cover.png", "image/png", b"plain text").unwrap_err();
        assert!(matches!(
            find_domain_error(&error),
            Some(DomainError::InvalidArgument(_))
        ));
        assert_eq!(
            error.to_string(),
            "image cover.png does not contain PNG, JPEG or WebP bytes"
        );
        let error = detect_media_kind("icon.svg", "image/svg+xml", b"<svg/>").unwrap_err();
        assert_eq!(error.to_string(), "unsupported image type for icon.svg");
        let error = detect_media_kind("cover.png", "image/png", b"\xff\xd8\xff").unwrap_err();
        assert_eq!(
            error.to_string(),
            "image cover.png declares image/png but contains image/jpeg bytes"
        );
    }

    #[test]
    fn plain_text_title_uses_txt_extension_and_media_type() {
        assert_eq!(
            text_filename_from_title("Runbook", LibraryTextContentFormat::PlainText),
            "Runbook.txt"
        );
        assert_eq!(
            text_media_type(LibraryTextContentFormat::PlainText),
            "text/plain"
        );
    }

    #[test]
    fn markdown_title_uses_md_extension_and_media_type() {
        assert_eq!(
            text_filename_from_title("Runbook", LibraryTextContentFormat::Markdown),
            "Runbook.md"
        );
        assert_eq!(
            text_media_type(LibraryTextContentFormat::Markdown),
            "text/markdown"
        );
    }

    #[test]
    fn markdown_replaces_trailing_txt_extension() {
        assert_eq!(
            text_filename_from_title("Runbook.txt", LibraryTextContentFormat::Markdown),
            "Runbook.md"
        );
    }
}
