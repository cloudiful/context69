//! Task enqueue handlers: the group-scoped batch endpoints, the generic
//! `POST /v1/tasks` submit, and the vector-index rebuild.

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use context69_contracts::{
    ApiErrorResponse, DeleteBatchRequest, FileBatchRequest, TaskRef, TaskSubmitRequest,
    TextBatchRequest, UrlBatchRequest,
};
use serde_json::json;

use super::{
    ApiState, CurrentUser, idempotency_key, managed_group, submit_task_request, task_error,
};
use crate::{
    api::auth::{AuthKind, RequestAuth},
    contracts::{PersonalAccessTokenScope, TaskKind, TokenGroupScope},
    services::tasks::TaskSubmission,
};

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/text", params(("group_path" = String, Path)), request_body = TextBatchRequest, responses((status = 202, description = "Text ingestion task accepted", body = TaskRef), (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse)))]
pub(crate) async fn submit_text_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<TextBatchRequest>,
) -> Response {
    let group = match managed_group(&state, &session, &group_path).await {
        Ok(group) => group,
        Err(response) => return *response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::TextBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/url", params(("group_path" = String, Path)), request_body = UrlBatchRequest, responses((status = 202, description = "URL ingestion task accepted", body = TaskRef), (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse)))]
pub(crate) async fn submit_url_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UrlBatchRequest>,
) -> Response {
    let group = match managed_group(&state, &session, &group_path).await {
        Ok(group) => group,
        Err(response) => return *response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::UrlBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/file", params(("group_path" = String, Path)), request_body = FileBatchRequest, responses((status = 202, description = "File ingestion task accepted", body = TaskRef), (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse)))]
pub(crate) async fn submit_file_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<FileBatchRequest>,
) -> Response {
    let group = match managed_group(&state, &session, &group_path).await {
        Ok(group) => group,
        Err(response) => return *response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::FileBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/delete", params(("group_path" = String, Path)), request_body = DeleteBatchRequest, responses((status = 202, description = "Document delete task accepted", body = TaskRef), (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse)))]
pub(crate) async fn submit_delete_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<DeleteBatchRequest>,
) -> Response {
    let group = match managed_group(&state, &session, &group_path).await {
        Ok(group) => group,
        Err(response) => return *response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/tasks", request_body = TaskSubmitRequest, responses((status = 202, description = "Submitted task accepted", body = TaskRef), (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse)))]
pub(crate) async fn submit_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Extension(auth): Extension<RequestAuth>,
    headers: HeaderMap,
    Json(request): Json<TaskSubmitRequest>,
) -> Response {
    let (kind, group_path, source_key, payloads) = match request {
        TaskSubmitRequest::RetryFileBatch { group_path, items } => (
            TaskKind::FileBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| json!({ "file_id": item.file_id }))
                .collect(),
        ),
        TaskSubmitRequest::FileBatch { group_path, items } => (
            TaskKind::FileBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("FileBatchItem is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::ImageBatch { group_path, items } => (
            TaskKind::ImageBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("ImageBatchItem is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::TextBatch { group_path, items } => (
            TaskKind::TextBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("text item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::UrlBatch { group_path, items } => (
            TaskKind::UrlBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("url item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::DeleteBatch { group_path, items } => (
            TaskKind::DeleteBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("delete item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::SourceSync {
            group_path,
            source_key,
        } => (
            TaskKind::SourceSync,
            group_path,
            Some(source_key),
            vec![json!({})],
        ),
        TaskSubmitRequest::TranslationBatch { group_path, items } => (
            TaskKind::Translation,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("translation item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::VectorRebuild => (TaskKind::VectorRebuild, None, None, vec![json!({})]),
    };
    if kind == TaskKind::VectorRebuild {
        let Some(authenticated) = auth.0.as_ref() else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse::new(
                    "unauthorized",
                    "missing authenticated session or personal access token".to_string(),
                )),
            )
                .into_response();
        };
        match global_vector_rebuild_admission(
            session.token_group_scope.as_ref(),
            &authenticated.kind,
        ) {
            GlobalVectorRebuildAdmission::Allowed => {}
            GlobalVectorRebuildAdmission::GroupRestricted => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiErrorResponse::new(
                        "forbidden",
                        "group-restricted personal access tokens cannot submit a global vector rebuild"
                            .to_string(),
                    )),
                )
                    .into_response();
            }
            GlobalVectorRebuildAdmission::MissingSettingsScope => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiErrorResponse::new(
                        "forbidden",
                        "personal access token missing settings scope".to_string(),
                    )),
                )
                    .into_response();
            }
        }
    }
    let group = if let Some(path) = group_path.as_deref() {
        match managed_group(&state, &session, path).await {
            Ok(group) => Some(group),
            Err(response) => return *response,
        }
    } else {
        None
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: group.as_ref().map(|value| value.id),
            group_path,
            source_key,
            kind,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/settings/runtime/vector-index/rebuild",
    responses((status = 202, description = "Vector index rebuild task accepted", body = TaskRef), (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse))
)]
pub(crate) async fn submit_vector_index_rebuild(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    headers: HeaderMap,
) -> Response {
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: None,
            group_path: None,
            source_key: None,
            kind: TaskKind::VectorRebuild,
            payloads: vec![json!({})],
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalVectorRebuildAdmission {
    Allowed,
    GroupRestricted,
    MissingSettingsScope,
}

/// The generic `/v1/tasks` entry point can carry the instance-wide vector
/// rebuild, so it must meet the canonical settings endpoint's settings scope.
/// Group-restricted tokens cannot name a group for this global operation and
/// are refused instead of silently widening to every group.
fn global_vector_rebuild_admission(
    token_group_scope: Option<&TokenGroupScope>,
    auth_kind: &AuthKind,
) -> GlobalVectorRebuildAdmission {
    if token_group_scope.is_some_and(|scope| scope.is_restricted()) {
        return GlobalVectorRebuildAdmission::GroupRestricted;
    }
    match auth_kind {
        AuthKind::BrowserSession => GlobalVectorRebuildAdmission::Allowed,
        AuthKind::PersonalAccessToken { scopes, .. }
            if scopes.contains(&PersonalAccessTokenScope::Settings) =>
        {
            GlobalVectorRebuildAdmission::Allowed
        }
        AuthKind::PersonalAccessToken { .. } => GlobalVectorRebuildAdmission::MissingSettingsScope,
    }
}

async fn submit(state: &ApiState, request: TaskSubmission) -> Response {
    submit_task_request(state, request).await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use uuid::Uuid;

    use super::{GlobalVectorRebuildAdmission, global_vector_rebuild_admission};
    use crate::{
        api::auth::AuthKind,
        contracts::{PersonalAccessTokenScope, TokenGroupScope},
    };

    fn token(scopes: &[PersonalAccessTokenScope]) -> AuthKind {
        AuthKind::PersonalAccessToken {
            token_id: Uuid::nil(),
            scopes: scopes.iter().copied().collect::<BTreeSet<_>>(),
        }
    }

    #[test]
    fn global_vector_rebuild_requires_settings_for_all_tokens_and_rejects_group_ranges() {
        assert_eq!(
            global_vector_rebuild_admission(None, &AuthKind::BrowserSession),
            GlobalVectorRebuildAdmission::Allowed
        );
        assert_eq!(
            global_vector_rebuild_admission(
                Some(&TokenGroupScope::All),
                &token(&[
                    PersonalAccessTokenScope::Workspace,
                    PersonalAccessTokenScope::Settings,
                ])
            ),
            GlobalVectorRebuildAdmission::Allowed
        );
        assert_eq!(
            global_vector_rebuild_admission(
                Some(&TokenGroupScope::All),
                &token(&[PersonalAccessTokenScope::Workspace])
            ),
            GlobalVectorRebuildAdmission::MissingSettingsScope
        );
        assert_eq!(
            global_vector_rebuild_admission(
                Some(&TokenGroupScope::Groups { group_ids: vec![7] }),
                &token(&[
                    PersonalAccessTokenScope::Workspace,
                    PersonalAccessTokenScope::Settings,
                ])
            ),
            GlobalVectorRebuildAdmission::GroupRestricted
        );
        assert_eq!(
            global_vector_rebuild_admission(
                Some(&TokenGroupScope::Groups { group_ids: vec![7] }),
                &token(&[PersonalAccessTokenScope::Workspace])
            ),
            GlobalVectorRebuildAdmission::GroupRestricted
        );
    }
}
