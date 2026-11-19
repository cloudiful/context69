use anyhow::Result;

use crate::{
    contracts::MembershipRole, domain::GroupRecord, domain_errors::DomainError,
    services::auth::AuthSession,
};

use super::ApiState;

pub(crate) async fn group_for_user(
    state: &ApiState,
    session: &AuthSession,
    group_path: &str,
) -> Result<GroupRecord> {
    let group = state
        .app
        .namespace
        .get_group_for_user(session.user.id, group_path, None)
        .await?
        .ok_or_else(|| DomainError::not_found("unknown group"))
        .map_err(anyhow::Error::from)?;
    crate::authorization::require_group_allowed(session.token_group_scope.as_ref(), group.id)?;
    Ok(group)
}

/// Decide one true group-role gate with the shared engine.
///
/// The engine receives the actual caller id, the actual group id, the live SQL
/// role and the token group ceiling in one call. The rank comparison is gone,
/// so this is the only permission decision for the gate.
pub(crate) fn require_group_role(
    state: &ApiState,
    session: &AuthSession,
    group: &GroupRecord,
    required: MembershipRole,
) -> Result<()> {
    group_action_allowed(
        &state.app.authorizer,
        session.user.id,
        group,
        required,
        session.token_group_scope.as_ref(),
    )
}

/// Decide one group-role gate against the shared engine.
///
/// The engine receives the actual caller id, the actual group id, the live SQL
/// role and the token group ceiling in one call. The rank comparison is gone,
/// so this is the only permission decision for the gate. It stays separate from
/// [`require_group_role`] so tests can exercise the real engine without an
/// application state.
pub(crate) fn group_action_allowed(
    authorizer: &context69_authz::Context69Authorizer,
    user_id: i64,
    group: &GroupRecord,
    required: MembershipRole,
    token_group_scope: Option<&context69_contracts::TokenGroupScope>,
) -> Result<()> {
    if authorizer.check(
        user_id,
        context69_authz::Operation::Group {
            group_id: group.id,
            required: context69_authz::GroupAction::for_role(required),
            current_role: group.current_role,
            token_scope: token_group_scope,
        },
    ) {
        Ok(())
    } else {
        Err(DomainError::forbidden("insufficient permissions for group").into())
    }
}

pub(crate) fn group_access_error_response(error: anyhow::Error) -> axum::response::Response {
    super::error_mapping::group_access_error_response(error)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::group_action_allowed;
    use crate::{
        contracts::{GroupKind, MembershipRole, TokenGroupScope, Visibility},
        domain::GroupRecord,
    };

    fn authorizer() -> context69_authz::Context69Authorizer {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(context69_authz::Context69Authorizer::new())
            .expect("engine")
    }

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

    fn check(
        group: &GroupRecord,
        required: MembershipRole,
        token_group_scope: Option<&TokenGroupScope>,
    ) -> anyhow::Result<()> {
        group_action_allowed(&authorizer(), 7, group, required, token_group_scope)
    }

    #[test]
    fn role_hierarchy_enforces_required_group_access() {
        let error = check(&group(None), MembershipRole::Maintainer, None)
            .expect_err("permission check should fail");
        assert_eq!(error.to_string(), "insufficient permissions for group");
        assert!(
            check(
                &group(Some(MembershipRole::Viewer)),
                MembershipRole::Maintainer,
                None,
            )
            .is_err()
        );
        assert!(
            check(
                &group(Some(MembershipRole::Maintainer)),
                MembershipRole::Maintainer,
                None,
            )
            .is_ok()
        );
        assert!(
            check(
                &group(Some(MembershipRole::Owner)),
                MembershipRole::Maintainer,
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn the_token_group_ceiling_must_admit_the_exact_group() {
        let allowed = TokenGroupScope::Groups { group_ids: vec![1] };
        let foreign = TokenGroupScope::Groups { group_ids: vec![2] };
        assert!(
            check(
                &group(Some(MembershipRole::Viewer)),
                MembershipRole::Viewer,
                Some(&allowed),
            )
            .is_ok(),
            "a token restricted to this group keeps the role right"
        );
        assert!(
            check(
                &group(Some(MembershipRole::Viewer)),
                MembershipRole::Viewer,
                Some(&foreign),
            )
            .is_err(),
            "a token restricted to another group cannot use this role right"
        );
        assert!(
            check(
                &group(Some(MembershipRole::Viewer)),
                MembershipRole::Viewer,
                Some(&TokenGroupScope::All),
            )
            .is_ok(),
            "an unrestricted token still needs the current role"
        );
    }

    #[test]
    fn group_role_errors_are_typed_forbidden() {
        let error = check(&group(None), MembershipRole::Maintainer, None).expect_err("should fail");
        let status = crate::domain_errors::status_for_error(&error);
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
    }
}
