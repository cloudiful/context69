use axum::{Json, http::StatusCode, response::IntoResponse};
use context69_contracts::ApiErrorCode;

use crate::contracts::ApiErrorResponse;
use crate::domain_errors::status_for_error;

pub(crate) fn error_response(status: StatusCode, message: String) -> ApiErrorResponse {
    let code = ApiErrorCode::code_for_status(status.as_u16());
    let canonical = context69_contracts::CanonicalApiErrorResponse::new(code, message);
    canonical.into()
}

fn json_response(status: StatusCode, message: String) -> axum::response::Response {
    (status, Json(error_response(status, message))).into_response()
}

fn typed_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = status_for_error(&error);
    json_response(status, message)
}

pub(crate) fn internal_error_response(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

pub(crate) fn source_management_error_response(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

pub(crate) fn library_management_error_response(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

pub(crate) fn admin_user_error_response(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

pub(crate) fn task_error(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

pub(crate) fn group_access_error_response(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

pub(crate) fn task_maintenance_error_response(error: anyhow::Error) -> axum::response::Response {
    typed_response(error)
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SourceKeyQuery {
    pub(crate) source_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_errors::DomainError;

    fn status_of(response: &axum::response::Response) -> StatusCode {
        response.status()
    }

    fn wrapped(error: DomainError) -> anyhow::Error {
        anyhow::Error::new(error).context("outer context")
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
    fn every_status_category_maps_via_typed_domain_error() {
        let cases = [
            (
                DomainError::invalid_argument("page_size must be between 1 and 100"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::unauthorized("invalid login or password"),
                StatusCode::UNAUTHORIZED,
            ),
            (
                DomainError::forbidden("admin access required"),
                StatusCode::FORBIDDEN,
            ),
            (
                DomainError::not_found("unknown group"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::conflict("source already exists"),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::payload_too_large("remote_file_too_large"),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                DomainError::unprocessable_entity("metadata_json must be an object"),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                DomainError::rate_limited("embedding 429"),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                DomainError::unavailable("s3 dependency unavailable"),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                DomainError::upstream_error("qdrant transport failed"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::upstream_timeout("remote_download_failed"),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                DomainError::internal("plain internal boom"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (error, status) in cases {
            for response in [
                internal_error_response(anyhow::Error::new(error.clone())),
                source_management_error_response(anyhow::Error::new(error.clone())),
                library_management_error_response(anyhow::Error::new(error.clone())),
                admin_user_error_response(anyhow::Error::new(error.clone())),
                task_error(anyhow::Error::new(error.clone())),
                group_access_error_response(anyhow::Error::new(error.clone())),
            ] {
                assert_eq!(status_of(&response), status, "message: {}", error.message());
            }
        }
    }

    #[test]
    fn nested_anyhow_contexts_preserve_typed_status() {
        let cases = [
            (
                DomainError::not_found("task not found"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::conflict("task is terminal"),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::forbidden("task management permission denied"),
                StatusCode::FORBIDDEN,
            ),
            (
                DomainError::conflict("task must be trashed before permanent deletion"),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::invalid_argument("task has no failed items to retry"),
                StatusCode::BAD_REQUEST,
            ),
        ];
        for (error, status) in cases {
            assert_eq!(status_of(&task_error(wrapped(error.clone()))), status);
            assert_eq!(
                status_of(&library_management_error_response(wrapped(error.clone()))),
                status
            );
        }
    }

    #[test]
    fn typed_task_error_preserves_legacy_statuses() {
        assert_eq!(
            status_of(&task_error(DomainError::not_found("task not found").into())),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(&task_error(
                DomainError::conflict("task is terminal").into()
            )),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(&task_error(
                DomainError::forbidden("task management permission denied").into()
            )),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_of(&task_error(
                DomainError::conflict("task must be trashed before permanent deletion").into()
            )),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(&task_error(
                DomainError::invalid_argument("task has no failed items to retry").into()
            )),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn library_metadata_object_maps_to_unprocessable_entity() {
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::unprocessable_entity("metadata_json must be an object").into()
            )),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::payload_too_large("remote_file_too_large").into()
            )),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::upstream_timeout("remote_download_failed").into()
            )),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::upstream_error("remote_dns_failed").into()
            )),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn group_access_maps_unknown_and_forbidden() {
        assert_eq!(
            status_of(&group_access_error_response(
                DomainError::not_found("unknown group foo").into()
            )),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(&group_access_error_response(
                DomainError::forbidden("insufficient permissions for group").into()
            )),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn dependency_and_upstream_errors_keep_typed_status_in_all_mappers() {
        let unavailable = DomainError::unavailable("s3 dependency unavailable");
        assert_eq!(
            status_of(&library_management_error_response(anyhow::Error::new(
                unavailable.clone()
            ))),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_of(&source_management_error_response(anyhow::Error::new(
                unavailable.clone()
            ))),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let timeout = DomainError::upstream_timeout("qdrant snapshot timed out");
        assert_eq!(
            status_of(&task_error(anyhow::Error::new(timeout.clone()))),
            StatusCode::GATEWAY_TIMEOUT
        );
        let rate = DomainError::rate_limited("embedding request failed: status=429");
        assert_eq!(
            status_of(&internal_error_response(anyhow::Error::new(rate))),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[test]
    fn auth_group_library_task_settings_translation_url_and_unknown_errors() {
        // auth: invalid credentials -> 401, disabled -> 401, admin required -> 403
        assert_eq!(
            status_of(&admin_user_error_response(
                DomainError::unauthorized("invalid login or password").into()
            )),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_of(&admin_user_error_response(
                DomainError::unauthorized("user account is disabled").into()
            )),
            StatusCode::UNAUTHORIZED
        );
        // group
        assert_eq!(
            status_of(&group_access_error_response(
                DomainError::not_found("unknown group").into()
            )),
            StatusCode::NOT_FOUND
        );
        // library + URL validation
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::invalid_argument("invalid_remote_url").into()
            )),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::invalid_argument("remote_url_blocked").into()
            )),
            StatusCode::BAD_REQUEST
        );
        // task
        assert_eq!(
            status_of(&task_error(DomainError::conflict("duplicate key").into())),
            StatusCode::CONFLICT
        );
        // settings validation
        assert_eq!(
            status_of(&internal_error_response(
                DomainError::invalid_argument("runtime.qdrant.url must not be empty").into()
            )),
            StatusCode::BAD_REQUEST
        );
        // translation validation
        assert_eq!(
            status_of(&library_management_error_response(
                DomainError::invalid_argument("locale must be a BCP 47 language tag").into()
            )),
            StatusCode::BAD_REQUEST
        );
        // unknown -> internal
        assert_eq!(
            status_of(&task_error(anyhow::anyhow!("plain internal boom"))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_of(&library_management_error_response(anyhow::anyhow!(
                "plain internal boom"
            ))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn migrated_producers_keep_typed_status() {
        let cases = [
            (
                DomainError::unauthorized("personal access token has been revoked"),
                StatusCode::UNAUTHORIZED,
            ),
            (
                DomainError::unauthorized("personal access token has expired"),
                StatusCode::UNAUTHORIZED,
            ),
            (
                DomainError::invalid_argument("token name must not be empty"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::forbidden("staged storage object belongs to another group"),
                StatusCode::FORBIDDEN,
            ),
            (
                DomainError::conflict("staged storage object uses inactive backend local"),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::not_found("unknown staged storage object 123"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::unavailable(
                    "s3 operation read failed after 3 attempts: kind=Unexpected: reset; s3 transient transport failure",
                ),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                DomainError::upstream_timeout("s3 operation read timed out after 30s"),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                DomainError::upstream_error("s3 operation read failed: kind=Unexpected: reset"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::not_found("s3 operation read failed: kind=NotFound: missing"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::upstream_error("fresh docling submission failed: gone"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::internal("Docling recovery completed but audit insertion failed: db"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            // document store + document query surface
            (
                DomainError::not_found("document not found"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::not_found("metadata index not found"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::invalid_argument("keys must contain 1..=200 items"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("limit must be between 1 and 200"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("sort supports at most 3 fields"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("invalid cursor"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("cursor does not match query"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("metadata field 'title' is not declared"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("metadata field 'title' is not ready"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument(
                    "source_key is required for metadata filters and sorting",
                ),
                StatusCode::BAD_REQUEST,
            ),
            // query runtime gating
            (
                DomainError::unavailable(
                    "vector index is rebuilding or unavailable; retry after the rebuild completes",
                ),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                DomainError::unavailable(
                    "search runtime is not configured; save runtime settings and restart the service",
                ),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            // embedding + qdrant upstream surface
            (
                DomainError::rate_limited("embedding request failed: status=429 kind=http"),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                DomainError::upstream_error("embedding request failed: status=503 kind=http"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::upstream_timeout(
                    "embedding upstream transport error: operation=send request kind=timeout",
                ),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                DomainError::payload_too_large("embedding response body exceeds 1024 bytes"),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                DomainError::upstream_error("qdrant transport failed: connection refused"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::upstream_timeout("qdrant search_points request timed out after 30s"),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            // file upload / staged content surface
            (
                DomainError::payload_too_large("file big.bin exceeds upload size limit"),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                DomainError::invalid_argument("sha256 must be 64 hexadecimal characters"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("unsupported file type for x.exe"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::not_found("unknown folder 123"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::conflict("staged storage object metadata does not match upload"),
                StatusCode::CONFLICT,
            ),
            // task creation / payload surface
            (
                DomainError::invalid_argument("task payload and input object counts do not match"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::conflict("idempotency key was already used with a different request"),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::not_found("unknown input storage object 123"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::forbidden("input storage object belongs to another group"),
                StatusCode::FORBIDDEN,
            ),
            // task item payload / stage surface
            (
                DomainError::invalid_argument("unsupported task kind wat"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("unsupported file task stage bogus"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::invalid_argument("translation tasks require group_id"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::not_found("source not found in task group"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::conflict("task item lease was lost while saving payload"),
                StatusCode::CONFLICT,
            ),
            // docling stage surface
            (
                DomainError::invalid_argument("plain text cannot be submitted to docling"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::upstream_error("Docling submission outcome is uncertain for item 1"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::upstream_timeout("docling conversion timed out: elapsed"),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                DomainError::payload_too_large("docling output exceeds maximum of 100 bytes"),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                DomainError::internal("docling is not configured"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                DomainError::not_found("stored file not found for file 123"),
                StatusCode::NOT_FOUND,
            ),
            // sync source surface
            (
                DomainError::unavailable("source origin is unavailable for src"),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ];
        for (error, status) in cases {
            assert_eq!(
                status_of(&library_management_error_response(anyhow::Error::new(
                    error.clone()
                ))),
                status,
                "message: {}",
                error.message()
            );
            assert_eq!(
                status_of(&task_error(anyhow::Error::new(error.clone()))),
                status,
                "message: {}",
                error.message()
            );
        }
    }

    #[test]
    fn task_maintenance_typed_and_domain_errors() {
        assert_eq!(
            status_of(&task_maintenance_error_response(
                DomainError::forbidden("admin access required").into()
            )),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_of(&task_maintenance_error_response(
                DomainError::conflict("active tasks must be cancelled before purging").into()
            )),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(&task_maintenance_error_response(anyhow::anyhow!(
                "plain internal boom"
            ))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn upload_size_branches_keep_disclosed_status() {
        // Intentional correction vs baseline single 500 `anyhow!("invalid
        // upload size")`: negative -> 400 InvalidArgument, oversize -> 413
        // PayloadTooLarge. Pins both branches through library and task mappers.
        let negative = DomainError::invalid_argument("invalid upload size -1");
        for response in [
            library_management_error_response(anyhow::Error::new(negative.clone())),
            task_error(anyhow::Error::new(negative.clone())),
        ] {
            assert_eq!(status_of(&response), StatusCode::BAD_REQUEST);
        }
        let oversize = DomainError::payload_too_large("invalid upload size 999999");
        for response in [
            library_management_error_response(anyhow::Error::new(oversize.clone())),
            task_error(anyhow::Error::new(oversize.clone())),
        ] {
            assert_eq!(status_of(&response), StatusCode::PAYLOAD_TOO_LARGE);
        }
    }

    #[test]
    fn outer_domain_error_context_preserves_status_in_mappers() {
        use anyhow::Context as _;
        for (typed, status) in [
            (
                DomainError::not_found("missing outer"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::invalid_argument("bad outer"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::internal("boom outer"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            let via_result: anyhow::Error = Err::<(), _>(anyhow::anyhow!("plain inner"))
                .context(typed.clone())
                .unwrap_err();
            assert_eq!(
                status_of(&library_management_error_response(via_result)),
                status
            );

            let via_option: anyhow::Error = None::<()>.context(typed.clone()).unwrap_err();
            assert_eq!(status_of(&task_error(via_option)), status);
        }
    }
}
