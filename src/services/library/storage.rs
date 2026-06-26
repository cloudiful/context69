use std::{fs, path::Path};

use sha2::{Digest, Sha256};

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
    if lowered.ends_with(".txt") || lowered.ends_with(".md") || media_type.starts_with("text/") {
        return Ok(LibraryFileKind::PlainText);
    }
    Err(anyhow!("unsupported file type for {}", filename))
}

pub(super) fn write_storage_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("failed to write file {}", path.display()))
}

pub(super) fn build_storage_rel_path(file_id: Uuid, filename: &str) -> String {
    let sanitized = filename
        .chars()
        .map(|ch| match ch {
            '/' | '\\' => '_',
            other => other,
        })
        .collect::<String>();
    format!("{file_id}/{sanitized}")
}

pub(super) fn text_filename_from_title(title: &str) -> String {
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
    if !stem.to_ascii_lowercase().ends_with(".txt") {
        stem.push_str(".txt");
    }
    stem
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
