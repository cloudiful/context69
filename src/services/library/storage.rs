use sha2::{Digest, Sha256};

use crate::contracts::LibraryTextContentFormat;

use super::*;

pub(super) fn detect_file_kind(filename: &str, media_type: &str) -> Result<LibraryFileKind> {
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
    Err(anyhow!("unsupported file type for {}", filename))
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

pub(super) fn hash_bytes(bytes: &[u8]) -> String {
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

    use super::{text_filename_from_title, text_media_type};

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
