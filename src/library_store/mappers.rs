use std::str::FromStr;

use anyhow::Result;

use super::{
    FileRow, FolderRow, JobRow, LibraryFileSummary, LibraryIngestJobResponse,
    LibraryPreviewContentFormat,
};
use crate::contracts::{LibraryIngestStatus, Visibility};
use crate::domain::{LibraryFileRecord, LibraryFolderRecord, LibraryIngestJobRecord};

pub(crate) fn infer_preview_content_format(
    filename: &str,
    media_type: &str,
) -> LibraryPreviewContentFormat {
    let lower_name = filename.to_ascii_lowercase();
    let lower_media = media_type.to_ascii_lowercase();

    if lower_media.starts_with("text/")
        || lower_media.contains("json")
        || lower_media.contains("xml")
        || lower_name.ends_with(".md")
        || lower_name.ends_with(".txt")
    {
        return LibraryPreviewContentFormat::PlainText;
    }

    LibraryPreviewContentFormat::Markdown
}

pub(super) fn folder_from_row(row: FolderRow) -> Result<LibraryFolderRecord> {
    Ok(LibraryFolderRecord {
        id: row.id,
        group_id: row.group_id,
        group_key: row.group_key,
        project_id: row.project_id,
        project_key: row.project_key,
        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
        parent_id: row.parent_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub(super) fn file_from_row(row: FileRow) -> Result<LibraryFileRecord> {
    Ok(LibraryFileRecord {
        id: row.id,
        group_id: row.group_id,
        group_key: row.group_key,
        project_id: row.project_id,
        project_key: row.project_key,
        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
        folder_id: row.folder_id,
        external_id: row.external_id,
        filename: row.filename,
        media_type: row.media_type,
        size_bytes: row.size_bytes,
        sha256: row.sha256,
        storage_rel_path: row.storage_rel_path,
        ingest_status: LibraryIngestStatus::from_str(&row.ingest_status)?,
        error_message: row.error_message,
        created_at: row.created_at,
        updated_at: row.updated_at,
        ingested_at: row.ingested_at,
    })
}

pub(super) fn job_from_row(row: JobRow) -> Result<LibraryIngestJobRecord> {
    Ok(LibraryIngestJobRecord {
        id: row.id,
        group_id: row.group_id,
        group_key: row.group_key,
        project_id: row.project_id,
        project_key: row.project_key,
        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
        file_id: row.file_id,
        status: LibraryIngestStatus::from_str(&row.status)?,
        docling_task_id: row.docling_task_id,
        error_message: row.error_message,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
    })
}

pub fn file_to_summary(file: &LibraryFileRecord) -> LibraryFileSummary {
    LibraryFileSummary {
        file_id: file.id,
        group_key: file.group_key.clone(),
        project_key: file.project_key.clone(),
        visibility: file.visibility,
        folder_id: file.folder_id,
        filename: file.filename.clone(),
        media_type: file.media_type.clone(),
        size_bytes: file.size_bytes,
        ingest_status: file.ingest_status,
        error_message: file.error_message.clone(),
        created_at: file.created_at,
        updated_at: file.updated_at,
        ingested_at: file.ingested_at,
    }
}

pub fn job_to_response(job: LibraryIngestJobRecord) -> LibraryIngestJobResponse {
    LibraryIngestJobResponse {
        job_id: job.id,
        group_key: job.group_key,
        project_key: job.project_key,
        visibility: job.visibility,
        file_id: job.file_id,
        status: job.status,
        docling_task_id: job.docling_task_id,
        error_message: job.error_message,
        created_at: job.created_at,
        started_at: job.started_at,
        finished_at: job.finished_at,
        updated_at: job.updated_at,
    }
}
