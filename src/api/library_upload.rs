use axum::{Json, extract::Multipart, http::StatusCode, response::IntoResponse};
use uuid::Uuid;

use crate::{
    contracts::ApiErrorResponse,
    services::library::UploadedLibraryFile,
};

pub(crate) async fn read_library_uploads(
    mut multipart: Multipart,
) -> Result<Vec<UploadedLibraryFile>, axum::response::Response> {
    let mut folder_id = None;
    let mut uploads = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response());
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
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(ApiErrorResponse {
                                error: format!("invalid folder_id: {error}"),
                            }),
                        )
                            .into_response());
                    }
                },
                Err(error) => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response());
                }
            }
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
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response());
            }
        };

        uploads.push(UploadedLibraryFile {
            folder_id,
            filename,
            media_type,
            bytes,
        });
    }

    if uploads.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "at least one file is required".to_string(),
            }),
        )
            .into_response());
    }

    Ok(uploads)
}
