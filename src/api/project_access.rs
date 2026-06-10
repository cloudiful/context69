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
        return Err(anyhow!("insufficient permissions for project"));
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::require_project_role;
    use crate::{
        contracts::{MembershipRole, Visibility},
        domain::ProjectRecord,
    };

    fn public_project(current_role: Option<MembershipRole>) -> ProjectRecord {
        ProjectRecord {
            id: 1,
            group_id: 1,
            group_key: "public".to_string(),
            project_key: "default-public".to_string(),
            name: "Default Public Project".to_string(),
            visibility: Visibility::Public,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            current_role,
        }
    }

    #[test]
    fn public_project_without_membership_returns_permission_error() {
        let error = require_project_role(&public_project(None), MembershipRole::Maintainer)
            .expect_err("permission check should fail");
        assert_eq!(error.to_string(), "insufficient permissions for project");
    }
}
