use std::collections::HashSet;

use anyhow::Result;
use uuid::Uuid;

use crate::contracts::LibraryTextContentFormat;
use crate::library_store::LibraryStore;

use super::storage;

pub(super) async fn resolve_project_text_filename(
    store: &LibraryStore,
    project_id: i64,
    folder_id: Option<Uuid>,
    exclude_file_id: Option<Uuid>,
    title: &str,
    format: LibraryTextContentFormat,
) -> Result<String> {
    let occupied = store
        .list_filenames_in_project_folder(project_id, folder_id, exclude_file_id)
        .await?;
    Ok(next_available_filename(
        &storage::text_filename_from_title(title, format),
        &occupied,
    ))
}

pub(super) async fn resolve_project_file_filename(
    store: &LibraryStore,
    project_id: i64,
    folder_id: Option<Uuid>,
    requested: &str,
) -> Result<String> {
    let occupied = store
        .list_filenames_in_project_folder(project_id, folder_id, None)
        .await?;
    Ok(next_available_filename(requested, &occupied))
}

fn next_available_filename(base_filename: &str, occupied: &[String]) -> String {
    let occupied = occupied.iter().cloned().collect::<HashSet<_>>();
    if !occupied.contains(base_filename) {
        return base_filename.to_string();
    }

    let (stem, extension) = split_filename(base_filename);
    for index in 2.. {
        let candidate = format!("{stem} ({index}){extension}");
        if !occupied.contains(&candidate) {
            return candidate;
        }
    }

    unreachable!("unbounded filename suffix search should always return");
}

fn split_filename(filename: &str) -> (&str, &str) {
    match filename.rsplit_once('.') {
        Some((stem, _extension)) if !stem.is_empty() => (stem, &filename[stem.len()..]),
        _ => (filename, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::next_available_filename;

    #[test]
    fn keeps_base_filename_when_unused() {
        let filename = next_available_filename("notice.txt", &[]);
        assert_eq!(filename, "notice.txt");
    }

    #[test]
    fn appends_numeric_suffix_for_first_collision() {
        let filename = next_available_filename("notice.txt", &["notice.txt".to_string()]);
        assert_eq!(filename, "notice (2).txt");
    }

    #[test]
    fn skips_to_next_available_suffix() {
        let filename = next_available_filename(
            "notice.txt",
            &["notice.txt".to_string(), "notice (2).txt".to_string()],
        );
        assert_eq!(filename, "notice (3).txt");
    }

    #[test]
    fn prefers_base_filename_when_only_suffix_is_taken() {
        let filename = next_available_filename("notice.txt", &["notice (2).txt".to_string()]);
        assert_eq!(filename, "notice.txt");
    }

    #[test]
    fn keeps_suffix_before_extension() {
        let filename = next_available_filename("notice.md.txt", &["notice.md.txt".to_string()]);
        assert_eq!(filename, "notice.md (2).txt");
    }

    #[test]
    fn collision_handles_dash_and_dot_separators() {
        let filename = next_available_filename(
            "report-2026.pdf",
            &[
                "report-2026.pdf".to_string(),
                "report-2026 (2).pdf".to_string(),
            ],
        );
        assert_eq!(filename, "report-2026 (3).pdf");
    }
}
