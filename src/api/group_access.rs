use anyhow::Result;

use crate::{contracts::MembershipRole, domain::GroupRecord, domain_errors::DomainError};

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
        .ok_or_else(|| DomainError::not_found("unknown group"))
        .map_err(anyhow::Error::from)
}

pub(crate) fn require_group_role(group: &GroupRecord, required: MembershipRole) -> Result<()> {
    let Some(actual) = group.current_role else {
        return Err(DomainError::forbidden("insufficient permissions for group").into());
    };
    if actual.rank() < required.rank() {
        return Err(DomainError::forbidden("insufficient permissions for group").into());
    }
    Ok(())
}

pub(crate) fn group_access_error_response(error: anyhow::Error) -> axum::response::Response {
    super::error_mapping::group_access_error_response(error)
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

    #[test]
    fn group_role_errors_are_typed_forbidden() {
        let error =
            require_group_role(&group(None), MembershipRole::Maintainer).expect_err("should fail");
        let status = crate::domain_errors::status_for_error(&error);
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
    }
}
