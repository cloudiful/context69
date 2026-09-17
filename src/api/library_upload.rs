use axum::extract::Multipart;
use context69_contracts::ApiErrorCode;
use uuid::Uuid;

use crate::services::library::UploadedLibraryFile;

fn invalid_argument(message: String) -> axum::response::Response {
    context69_http_support::json_error_for_code(ApiErrorCode::InvalidArgument, message)
}

fn unprocessable_entity(message: String) -> axum::response::Response {
    context69_http_support::json_error_for_code(ApiErrorCode::UnprocessableEntity, message)
}

#[derive(Debug)]
enum ParseIngestOptionsError {
    InvalidArgument(String),
    UnprocessableEntity(String),
}

/// Parse the multipart `metadata` field as canonical
/// [`context69_contracts::IngestOptions`] only. Flattened v0.15 payloads are
/// rejected (breaking v0.18): unknown fields fail deserialization, and a
/// non-object `metadata_json` yields the preserved 422
/// `metadata_json must be an object`.
fn parse_multipart_ingest_options(
    raw: &[u8],
) -> Result<context69_contracts::IngestOptions, ParseIngestOptionsError> {
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|error| {
        ParseIngestOptionsError::InvalidArgument(format!("invalid metadata JSON: {error}"))
    })?;
    match serde_json::from_value::<context69_contracts::IngestOptions>(value.clone()) {
        Ok(options) => Ok(options),
        Err(error) => {
            let non_object = value
                .pointer("/metadata/metadata_json")
                .is_some_and(|v| !v.is_object());
            if non_object {
                return Err(ParseIngestOptionsError::UnprocessableEntity(
                    "metadata_json must be an object".to_string(),
                ));
            }
            Err(ParseIngestOptionsError::InvalidArgument(format!(
                "invalid metadata JSON: {error}"
            )))
        }
    }
}

pub(crate) async fn read_library_uploads(
    mut multipart: Multipart,
) -> Result<Vec<UploadedLibraryFile>, Box<axum::response::Response>> {
    let mut folder_id = None;
    let mut uploads = Vec::new();
    let mut declared_sha256 = None;
    let mut options: Option<context69_contracts::IngestOptions> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                return Err(Box::new(invalid_argument(error.to_string())));
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
                        return Err(Box::new(invalid_argument(format!("invalid folder_id: {error}"))));
                    }
                },
                Err(error) => {
                    return Err(Box::new(invalid_argument(error.to_string())));
                }
            }
            continue;
        }

        if name == "sha256" {
            declared_sha256 = match field.text().await {
                Ok(value) => Some(value.trim().to_ascii_lowercase()),
                Err(error) => {
                    return Err(Box::new(invalid_argument(error.to_string())));
                }
            };
            continue;
        }

        if name == "metadata" {
            options = match field.bytes().await {
                Ok(value) => match parse_multipart_ingest_options(&value) {
                    Ok(value) => Some(value),
                    Err(ParseIngestOptionsError::InvalidArgument(message)) => {
                        return Err(Box::new(invalid_argument(message)));
                    }
                    Err(ParseIngestOptionsError::UnprocessableEntity(message)) => {
                        return Err(Box::new(unprocessable_entity(message)));
                    }
                },
                Err(error) => {
                    return Err(Box::new(invalid_argument(error.to_string())));
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
                return Err(Box::new(invalid_argument(error.to_string())));
            }
        };

        let resolved = options.clone().unwrap_or_default();
        uploads.push(UploadedLibraryFile {
            folder_id,
            filename,
            media_type,
            bytes,
            declared_sha256: declared_sha256.take(),
            options: resolved,
            staged_storage_object_id: None,
        });
    }

    if uploads.is_empty() {
        return Err(Box::new(invalid_argument(
            "at least one file is required".to_string(),
        )));
    }

    Ok(uploads)
}

#[cfg(test)]
mod tests {
    use super::{ParseIngestOptionsError, parse_multipart_ingest_options};

    #[test]
    fn multipart_accepts_canonical_ingest_options() {
        let raw = serde_json::json!({
            "metadata": {
                "external_id": "doc-1",
                "metadata_json": { "agency": "x" }
            },
            "source_policy": "release_after_processing"
        });
        let bytes = serde_json::to_vec(&raw).expect("serialize canonical");
        let options = parse_multipart_ingest_options(&bytes).expect("canonical parses");
        assert!(options.is_release());
        assert_eq!(options.metadata.external_id.as_deref(), Some("doc-1"));
        assert_eq!(
            options.metadata.metadata_json.get("agency"),
            Some(&serde_json::json!("x"))
        );
    }

    #[test]
    fn multipart_rejects_legacy_flattened_payload() {
        let raw = serde_json::json!({
            "external_id": "doc-1",
            "metadata_json": { "agency": "x" },
            "delete_source_after_processing": true
        });
        let bytes = serde_json::to_vec(&raw).expect("serialize legacy");
        match parse_multipart_ingest_options(&bytes) {
            Err(ParseIngestOptionsError::InvalidArgument(_)) => {}
            other => panic!("legacy flattened payload must be rejected, got {other:?}"),
        }
    }

    #[test]
    fn multipart_preserves_metadata_object_message_for_canonical() {
        let raw = serde_json::json!({
            "metadata": { "metadata_json": [1, 2] },
            "source_policy": "retain"
        });
        let bytes = serde_json::to_vec(&raw).expect("serialize bad");
        match parse_multipart_ingest_options(&bytes) {
            Err(ParseIngestOptionsError::UnprocessableEntity(message)) => {
                assert_eq!(message, "metadata_json must be an object");
            }
            other => panic!("expected 422 for {raw}, got {other:?}"),
        }
    }

    #[test]
    fn multipart_rejects_malformed_json_as_invalid_argument() {
        match parse_multipart_ingest_options(b"{not json") {
            Err(ParseIngestOptionsError::InvalidArgument(message)) => {
                assert!(message.starts_with("invalid metadata JSON: "));
            }
            other => panic!("expected 400, got {other:?}"),
        }
    }
}
