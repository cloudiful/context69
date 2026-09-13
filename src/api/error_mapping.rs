use axum::{Json, http::StatusCode, response::IntoResponse};
use context69_contracts::ApiErrorCode;

use crate::contracts::ApiErrorResponse;
use crate::services::tasks::TaskMaintenanceError;

pub(crate) fn error_response(status: StatusCode, message: String) -> ApiErrorResponse {
    let code = ApiErrorCode::code_for_status(status.as_u16());
    ApiErrorResponse::new(code.as_str(), message)
}

fn json_response(status: StatusCode, message: String) -> axum::response::Response {
    (status, Json(error_response(status, message))).into_response()
}

fn runtime_aware_status(message: &str) -> Option<StatusCode> {
    context69_http_support::runtime_aware_status(message)
}

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
    json_response(status, message)
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
    json_response(status, message)
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
        || message.contains("cannot release its source")
        || message.contains("release their source")
        || message.contains("released file source cannot be reprocessed")
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
    json_response(status, message)
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
    json_response(status, message)
}

pub(crate) fn task_error(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if let Some(status) = runtime_aware_status(&message) {
        status
    } else if message.contains("not found") || message.contains("unknown group") {
        StatusCode::NOT_FOUND
    } else if message.contains("conflict")
        || message.contains("duplicate key")
        || message.contains("already used")
        || message.contains("terminal")
        || message.contains("cannot be trashed")
        || message.contains("must be trashed")
    {
        StatusCode::CONFLICT
    } else if message.contains("permission") {
        StatusCode::FORBIDDEN
    } else if message.contains("must")
        || message.contains("requires")
        || message.contains("no retryable")
        || message.contains("no failed items to retry")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    json_response(status, message)
}

pub(crate) fn group_access_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("unknown group") {
        StatusCode::NOT_FOUND
    } else if message.contains("insufficient permissions") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    json_response(status, message)
}

pub(crate) fn task_maintenance_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let recovery_error = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<TaskMaintenanceError>());
    let status = if let Some(recovery_error) = recovery_error {
        match recovery_error {
            TaskMaintenanceError::BadRequest(_) => StatusCode::BAD_REQUEST,
            TaskMaintenanceError::Conflict(_) => StatusCode::CONFLICT,
            TaskMaintenanceError::NotFound(_) => StatusCode::NOT_FOUND,
        }
    } else if message.contains("admin access required") {
        StatusCode::FORBIDDEN
    } else if message.contains("must be cancelled") {
        StatusCode::CONFLICT
    } else if message.contains("must be between") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    json_response(status, message)
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SourceKeyQuery {
    pub(crate) source_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_of(response: &axum::response::Response) -> StatusCode {
        response.status()
    }

    #[test]
    fn error_code_matches_status_for_all_variants() {
        for (code, status) in [
            (ApiErrorCode::InvalidArgument, 400),
            (ApiErrorCode::Unauthorized, 401),
            (ApiErrorCode::Forbidden, 403),
            (ApiErrorCode::NotFound, 404),
            (ApiErrorCode::Conflict, 409),
            (ApiErrorCode::PayloadTooLarge, 413),
            (ApiErrorCode::UnprocessableEntity, 422),
            (ApiErrorCode::RateLimited, 429),
            (ApiErrorCode::UpstreamError, 502),
            (ApiErrorCode::Unavailable, 503),
            (ApiErrorCode::UpstreamTimeout, 504),
            (ApiErrorCode::Internal, 500),
        ] {
            assert_eq!(code.status_code(), status);
            assert_eq!(ApiErrorCode::code_for_status(status), code);
            let body = error_response(
                StatusCode::from_u16(status).expect("status"),
                "boom".to_string(),
            );
            assert_eq!(body.code, code.as_str());
            let parsed: ApiErrorCode = body.code.parse().expect("code parses");
            assert_eq!(parsed, code);
        }
    }

    #[test]
    fn typed_task_error_preserves_legacy_statuses() {
        assert_eq!(
            status_of(&task_error(anyhow::anyhow!("task not found"))),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(&task_error(anyhow::anyhow!("task is terminal"))),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(&task_error(anyhow::anyhow!(
                "task management permission denied"
            ))),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_of(&task_error(anyhow::anyhow!(
                "task must be trashed before permanent deletion"
            ))),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(&task_error(anyhow::anyhow!(
                "task has no failed items to retry"
            ))),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn library_metadata_object_maps_to_unprocessable_entity() {
        assert_eq!(
            status_of(&library_management_error_response(anyhow::anyhow!(
                "metadata_json must be an object"
            ))),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status_of(&library_management_error_response(anyhow::anyhow!(
                "remote_file_too_large"
            ))),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            status_of(&library_management_error_response(anyhow::anyhow!(
                "remote_download_failed"
            ))),
            StatusCode::GATEWAY_TIMEOUT
        );
    }

    #[test]
    fn group_access_maps_unknown_and_forbidden() {
        assert_eq!(
            status_of(&group_access_error_response(anyhow::anyhow!(
                "unknown group foo"
            ))),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(&group_access_error_response(anyhow::anyhow!(
                "insufficient permissions for group"
            ))),
            StatusCode::FORBIDDEN
        );
    }
}
