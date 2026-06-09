use anyhow::{Context, Result, anyhow};
use axum::{Json, http::StatusCode, response::IntoResponse};

use crate::{
    contracts::{ApiErrorResponse, MembershipRole},
    domain::ProjectRecord,
};

use super::ApiState;

pub(crate) async fn project_for_user(
    state: &ApiState,
    user_id: i64,
    group_key: &str,
    project_key: &str,
) -> Result<ProjectRecord> {
    state
        .app
        .db
        .get_project_for_user(user_id, group_key, project_key)
        .await?
        .context("unknown project")
}

pub(crate) fn require_project_role(
    project: &ProjectRecord,
    required: MembershipRole,
) -> Result<()> {
    let Some(actual) = project.current_role else {
        return Err(anyhow!("unknown project"));
    };
    if actual.rank() < required.rank() {
        return Err(anyhow!("insufficient permissions for project"));
    }
    Ok(())
}

pub(crate) fn project_access_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("unknown project") {
        StatusCode::NOT_FOUND
    } else if message.contains("insufficient permissions") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiErrorResponse { error: message })).into_response()
}
