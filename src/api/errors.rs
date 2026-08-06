use axum::{Json, http::StatusCode, response::IntoResponse};

use crate::contracts::ApiErrorResponse;

pub(crate) fn internal_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("page must be")
        || message.contains("page_size must be")
        || message.contains("page offset is too large")
    {
        StatusCode::BAD_REQUEST
    } else {
        runtime_aware_status(&message).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    };
    (status, Json(error_response(status, message))).into_response()
}

pub(crate) fn source_management_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if let Some(status) = runtime_aware_status(&message) {
        status
    } else if message.contains("already exists") {
        StatusCode::CONFLICT
    } else if message.contains("unknown source") {
        StatusCode::NOT_FOUND
    } else if message.contains("must not be empty")
        || message.contains("cannot be changed")
        || message.contains("batch_size must")
        || message.contains("unsupported")
        || message.contains("unknown connection")
        || message.contains("failed to validate source")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(error_response(status, message))).into_response()
}

pub(crate) fn library_management_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if let Some(status) = runtime_aware_status(&message) {
        status
    } else if message.contains("processing job management requires") {
        StatusCode::FORBIDDEN
    } else if message.contains("unknown folder")
        || message.contains("unknown file")
        || message.contains("unknown job")
        || message.contains("unknown target folder")
        || message.contains("stored file not found for file")
        || message.contains("unknown URL import job")
        || message.contains("translation job not found")
        || message.contains("translation document not found")
    {
        StatusCode::NOT_FOUND
    } else if message.contains("external_id_content_conflict")
        || message.contains("cannot be retried")
        || message.contains("translation job is not retryable")
    {
        StatusCode::CONFLICT
    } else if message.contains("metadata_json must be an object")
        || message.contains("metadata field '")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if message.contains("must not be empty")
        || message.contains("unsupported file type")
        || message.contains("cannot be moved")
        || message.contains("folder name")
        || message.contains("invalid folder_id")
        || message.contains("exceeds upload size limit")
        || message.contains("page must be")
        || message.contains("page_size must be")
        || message.contains("page offset is too large")
        || message.contains("duplicate key value")
        || message.contains("invalid_remote_url")
        || message.contains("remote_url_blocked")
        || message.contains("remote_filename_required")
        || message.contains("translation provider")
        || message.contains("monthly character limit")
        || message.contains("locale must")
        || message.contains("locale region")
        || message.contains("glossary requires")
    {
        StatusCode::BAD_REQUEST
    } else if message.contains("remote_file_too_large") {
        StatusCode::PAYLOAD_TOO_LARGE
    } else if message.contains("remote_download_failed") {
        StatusCode::GATEWAY_TIMEOUT
    } else if message.contains("remote_") {
        StatusCode::BAD_GATEWAY
    } else if message.contains("is not failed and cannot be retried") {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(error_response(status, message))).into_response()
}

pub(crate) fn admin_user_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if let Some(status) = runtime_aware_status(&message) {
        status
    } else if message.contains("admin access required") {
        StatusCode::FORBIDDEN
    } else if message.contains("user not found") {
        StatusCode::NOT_FOUND
    } else if message.contains("must not be empty")
        || message.contains("last administrator")
        || message.contains("user account is disabled")
    {
        StatusCode::BAD_REQUEST
    } else if message.contains("duplicate key value") || message.contains("already exists") {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(error_response(status, message))).into_response()
}

pub(crate) fn error_response(status: StatusCode, message: String) -> ApiErrorResponse {
    ApiErrorResponse::new(ApiErrorResponse::code_for_status(status.as_u16()), message)
}

fn runtime_aware_status(message: &str) -> Option<StatusCode> {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("s3 dependency unavailable")
        || normalized.contains("docling dependency unavailable")
        || normalized.contains("embedding/vector dependency unavailable")
        || normalized.contains("library dependency unavailable")
        || normalized.contains("runtime is not configured")
        || normalized.contains("docling is not configured")
        || message.contains("save runtime settings and restart the service")
    {
        Some(StatusCode::SERVICE_UNAVAILABLE)
    } else if normalized.contains("embedding upstream transport error") {
        if normalized.contains("kind=timeout") || normalized.contains("timed out") {
            Some(StatusCode::GATEWAY_TIMEOUT)
        } else {
            Some(StatusCode::BAD_GATEWAY)
        }
    } else if normalized.contains("embedding request failed: status=429") {
        Some(StatusCode::TOO_MANY_REQUESTS)
    } else if normalized.contains("qdrant")
        && (normalized.contains("timeout") || normalized.contains("timed out"))
    {
        Some(StatusCode::GATEWAY_TIMEOUT)
    } else if normalized.contains("qdrant") && normalized.contains("429") {
        Some(StatusCode::TOO_MANY_REQUESTS)
    } else if normalized.contains("qdrant")
        && (normalized.contains("transport")
            || normalized.contains("connect")
            || normalized.contains("connection"))
    {
        Some(StatusCode::BAD_GATEWAY)
    } else if normalized.contains("embedding request failed:")
        || normalized.contains("failed to parse embedding response:")
    {
        Some(StatusCode::BAD_GATEWAY)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::runtime_aware_status;

    #[test]
    fn maps_embedding_upstream_failures() {
        assert_eq!(
            runtime_aware_status(
                "embedding upstream transport error: operation=read response body kind=timeout"
            ),
            Some(StatusCode::GATEWAY_TIMEOUT)
        );
        assert_eq!(
            runtime_aware_status(
                "embedding upstream transport error: operation=send request kind=connect"
            ),
            Some(StatusCode::BAD_GATEWAY)
        );
        assert_eq!(
            runtime_aware_status("embedding request failed: status=429 Too Many Requests"),
            Some(StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(
            runtime_aware_status("embedding request failed: status=401 Unauthorized"),
            Some(StatusCode::BAD_GATEWAY)
        );
    }

    #[test]
    fn maps_non_failed_retry_to_conflict() {
        assert_eq!(
            super::library_management_error_response(anyhow::anyhow!(
                "file id is not failed and cannot be retried"
            ))
            .status(),
            StatusCode::CONFLICT
        );
    }
}
