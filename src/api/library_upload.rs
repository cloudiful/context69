use axum::extract::Multipart;
use context69_contracts::ApiErrorCode;
use uuid::Uuid;

use crate::{contracts::LibraryFileIngestOptions, services::library::UploadedLibraryFile};

fn invalid_argument(message: String) -> axum::response::Response {
    context69_http_support::json_error_for_code(ApiErrorCode::InvalidArgument, message)
}

fn unprocessable_entity(message: String) -> axum::response::Response {
    context69_http_support::json_error_for_code(ApiErrorCode::UnprocessableEntity, message)
}

pub(crate) async fn read_library_uploads(
    mut multipart: Multipart,
) -> Result<Vec<UploadedLibraryFile>, axum::response::Response> {
    let mut folder_id = None;
    let mut uploads = Vec::new();
    let mut declared_sha256 = None;
    let mut options = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                return Err(invalid_argument(error.to_string()));
            }
        };
        let Some(field) = field else {
            break;
        };

        let name = field.name().unwrap_or_default().to_string();
        if name == "folder_id" {
            match field.text().await {
                Ok(text) if text.trim().is_empty() => {}
                Ok(text) => match Uuid::parse_str(text.trim()) {
                    Ok(value) => folder_id = Some(value),
                    Err(error) => {
                        return Err(invalid_argument(format!("invalid folder_id: {error}")));
                    }
                },
                Err(error) => {
                    return Err(invalid_argument(error.to_string()));
                }
            }
            continue;
        }

        if name == "sha256" {
            declared_sha256 = match field.text().await {
                Ok(value) => Some(value.trim().to_ascii_lowercase()),
                Err(error) => {
                    return Err(invalid_argument(error.to_string()));
                }
            };
            continue;
        }

        if name == "metadata" {
            options = match field.bytes().await {
                Ok(value) => match serde_json::from_slice::<LibraryFileIngestOptions>(&value) {
                    Ok(value) => {
                        if crate::contracts::strict_metadata_object(&value.metadata.metadata_json)
                            .is_err()
                        {
                            return Err(unprocessable_entity(
                                "metadata_json must be an object".to_string(),
                            ));
                        }
                        Some(value)
                    }
                    Err(error) => {
                        return Err(invalid_argument(format!("invalid metadata JSON: {error}")));
                    }
                },
                Err(error) => {
                    return Err(invalid_argument(error.to_string()));
                }
            };
            continue;
        }

        if name != "files" {
            continue;
        }

        let filename = field
            .file_name()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "upload.bin".to_string());
        let media_type = field
            .content_type()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(invalid_argument(error.to_string()));
            }
        };

        uploads.push(UploadedLibraryFile {
            folder_id,
            filename,
            media_type,
            bytes,
            declared_sha256: declared_sha256.take(),
            metadata: options.as_ref().map(|value| value.metadata.clone()),
            translation: options.as_ref().and_then(|value| value.translation.clone()),
            extraction: options.as_ref().and_then(|value| value.extraction.clone()),
            staged_storage_object_id: None,
            delete_source_after_processing: options
                .as_ref()
                .map(|value| {
                    context69_contracts::SourcePolicy::from_delete_flag(
                        value.delete_source_after_processing,
                    )
                    .as_delete_flag()
                })
                .unwrap_or(false),
        });
    }

    if uploads.is_empty() {
        return Err(invalid_argument(
            "at least one file is required".to_string(),
        ));
    }

    Ok(uploads)
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonical_ingest_options_preserves_source_policy() {
        use context69_contracts::SourcePolicy;
        assert_eq!(SourcePolicy::from_delete_flag(false), SourcePolicy::Retain);
        assert_eq!(
            SourcePolicy::from_delete_flag(true),
            SourcePolicy::ReleaseAfterProcessing
        );
        let options = context69_contracts::IngestOptions::from_legacy(None, None, None, true);
        assert!(options.is_release());
        assert!(options.as_delete_flag());
    }

    #[test]
    fn strict_metadata_object_rejects_non_objects_at_boundary() {
        assert!(crate::contracts::strict_metadata_object(&serde_json::json!([1, 2])).is_err());
        assert!(crate::contracts::strict_metadata_object(&serde_json::json!({ "k": "v" })).is_ok());
    }
}
