use anyhow::{Context, Result, anyhow};
use axum::{Json, http::StatusCode, response::IntoResponse};

use crate::{
    contracts::{ApiErrorResponse, MembershipRole},
    domain::GroupRecord,
};

use super::ApiState;

pub(crate) async fn group_for_user(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> Result<GroupRecord> {
    state
        .app
        .namespace
        .get_group_for_user(user_id, group_path)
        .await?
        .context("unknown group")
}

pub(crate) fn require_group_role(group: &GroupRecord, required: MembershipRole) -> Result<()> {
    let Some(actual) = group.current_role else {
        return Err(anyhow!("insufficient permissions for group"));
    };
    if actual.rank() < required.rank() {
        return Err(anyhow!("insufficient permissions for group"));
    }
    Ok(())
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

    (
        status,
        Json(ApiErrorResponse::new(
            ApiErrorResponse::code_for_status(status.as_u16()),
            message,
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::require_group_role;
    use crate::{
        contracts::{GroupKind, MembershipRole, Visibility},
        domain::GroupRecord,
    };

    fn group(current_role: Option<MembershipRole>) -> GroupRecord {
        GroupRecord {
            id: 1,
            parent_group_id: None,
            group_path: "public".to_string(),
            parent_group_path: None,
            group_key: "public".to_string(),
            name: "Public".to_string(),
            visibility: Visibility::Public,
            kind: GroupKind::Shared,
            owner_user_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            current_role,
        }
    }

    #[test]
    fn role_hierarchy_enforces_required_group_access() {
        let error = require_group_role(&group(None), MembershipRole::Maintainer)
            .expect_err("permission check should fail");
        assert_eq!(error.to_string(), "insufficient permissions for group");
        assert!(
            require_group_role(
                &group(Some(MembershipRole::Viewer)),
                MembershipRole::Maintainer,
            )
            .is_err()
        );
        assert!(
            require_group_role(
                &group(Some(MembershipRole::Maintainer)),
                MembershipRole::Maintainer,
            )
            .is_ok()
        );
        assert!(
            require_group_role(
                &group(Some(MembershipRole::Owner)),
                MembershipRole::Maintainer
            )
            .is_ok()
        );
    }
}
