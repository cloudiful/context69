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

/// Parse the multipart `metadata` field as canonical [`context69_contracts::IngestOptions`]
/// when it carries v0.16 keys (`source_policy` or an object `metadata`), else as the
/// v0.15 flattened [`crate::contracts::LibraryFileIngestOptions`].
///
/// v0.15 payloads stay readable: legacy JSON without `options` keeps working, and a
/// non-object `metadata_json` still yields the preserved 422
/// `metadata_json must be an object` in both shapes.
fn parse_multipart_ingest_options(
    raw: &[u8],
) -> Result<context69_contracts::IngestOptions, ParseIngestOptionsError> {
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|error| {
        ParseIngestOptionsError::InvalidArgument(format!("invalid metadata JSON: {error}"))
    })?;
    let is_canonical = value.get("source_policy").is_some()
        || value.get("metadata").is_some_and(|v| v.is_object());
    if is_canonical {
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
    } else {
        match serde_json::from_value::<crate::contracts::LibraryFileIngestOptions>(value) {
            Ok(legacy) => {
                if crate::contracts::strict_metadata_object(&legacy.metadata.metadata_json).is_err()
                {
                    return Err(ParseIngestOptionsError::UnprocessableEntity(
                        "metadata_json must be an object".to_string(),
                    ));
                }
                Ok(legacy.ingest_options())
            }
            Err(error) => Err(ParseIngestOptionsError::InvalidArgument(format!(
                "invalid metadata JSON: {error}"
            ))),
        }
    }
}

pub(crate) async fn read_library_uploads(
    mut multipart: Multipart,
) -> Result<Vec<UploadedLibraryFile>, axum::response::Response> {
    let mut folder_id = None;
    let mut uploads = Vec::new();
    let mut declared_sha256 = None;
    let mut options: Option<context69_contracts::IngestOptions> = None;

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
                Ok(value) => match parse_multipart_ingest_options(&value) {
                    Ok(value) => Some(value),
                    Err(ParseIngestOptionsError::InvalidArgument(message)) => {
                        return Err(invalid_argument(message));
                    }
                    Err(ParseIngestOptionsError::UnprocessableEntity(message)) => {
                        return Err(unprocessable_entity(message));
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

        let resolved = options.clone().unwrap_or_default();
        let legacy_metadata: context69_contracts::LibraryFileUploadMetadata =
            resolved.metadata.clone().into();
        let has_legacy_metadata = legacy_metadata.external_id.is_some()
            || legacy_metadata.source_uri.is_some()
            || legacy_metadata.published_at.is_some()
            || legacy_metadata
                .metadata_json
                .as_object()
                .is_some_and(|map| !map.is_empty());
        uploads.push(UploadedLibraryFile {
            folder_id,
            filename,
            media_type,
            bytes,
            declared_sha256: declared_sha256.take(),
            metadata: if has_legacy_metadata {
                Some(legacy_metadata)
            } else {
                None
            },
            translation: resolved.translation.clone(),
            extraction: resolved.extraction.clone(),
            staged_storage_object_id: None,
            delete_source_after_processing: resolved.as_delete_flag(),
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
    use super::{ParseIngestOptionsError, parse_multipart_ingest_options};

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
    fn multipart_keeps_legacy_flattened_payload_readable() {
        let raw = serde_json::json!({
            "external_id": "doc-1",
            "metadata_json": { "agency": "x" },
            "delete_source_after_processing": true
        });
        let bytes = serde_json::to_vec(&raw).expect("serialize legacy");
        let options = parse_multipart_ingest_options(&bytes).expect("legacy parses");
        assert!(options.is_release());
        assert_eq!(options.metadata.external_id.as_deref(), Some("doc-1"));
    }

    #[test]
    fn multipart_preserves_metadata_object_message_for_both_shapes() {
        for raw in [
            serde_json::json!({
                "external_id": "doc-1",
                "metadata_json": [1, 2],
                "delete_source_after_processing": false
            }),
            serde_json::json!({
                "metadata": { "metadata_json": [1, 2] },
                "source_policy": "retain"
            }),
        ] {
            let bytes = serde_json::to_vec(&raw).expect("serialize bad");
            match parse_multipart_ingest_options(&bytes) {
                Err(ParseIngestOptionsError::UnprocessableEntity(message)) => {
                    assert_eq!(message, "metadata_json must be an object");
                }
                other => panic!("expected 422 for {raw}, got {other:?}"),
            }
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
