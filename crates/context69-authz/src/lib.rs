//! Context69 adapter over the shared `cloudiful-authz` decision engine.
//!
//! One [`Context69Authorizer`] is built once at application startup. It owns the
//! fixed role schema for the `context69` service domain (the administrator
//! control role and the six personal-access-token scopes) and the group role
//! schema (viewer, maintainer, owner). Live facts are supplied per request:
//! the real user id, the current administrator flag, the current SQL-resolved
//! group role and the caller's personal-access-token facts. Dynamic group
//! domains are never registered, so a new group works without rebuilding the
//! engine.
//!
//! The adapter never manufactures a principal: anonymous callers stay outside
//! the engine, and a non-positive or synthetic id is refused.

use std::collections::BTreeSet;

use cloudiful_authz::{
    Action, Attenuation, Authorizer, AuthorizerConfig, AuthzRequest, Capability, CredentialContext,
    Decision, DomainHierarchy, DomainId, FactsRevision, Grant, PrincipalId, PrincipalKind,
    PrincipalRef, ResolvedRoles, ResourceKind, ResourceRef, RoleId, RoleSpec, ServiceId, Subject,
};
use context69_contracts_auth::PersonalAccessTokenScope;
use context69_contracts_core::TokenGroupScope;
use context69_contracts_namespace::MembershipRole;

pub use cloudiful_authz::InitError;

/// The one service domain for global administration and the six PAT scopes.
const SERVICE_DOMAIN: &str = "context69";
/// The receiving service audience. It is separate from every permission domain.
const SERVICE_AUDIENCE: &str = "context69";

const SCOPE_RESOURCE: &str = "scope";
const ADMIN_RESOURCE: &str = "admin";
const ADMIN_MANAGE_ACTION: &str = "manage";
const GROUP_RESOURCE: &str = "group";
const GROUP_VIEW_ACTION: &str = "view";
const GROUP_CONTRIBUTE_ACTION: &str = "contribute";
const GROUP_MANAGE_ACTION: &str = "manage";

const ADMIN_ROLE: &str = "admin";
const AUTHENTICATED_ROLE: &str = "authenticated";
const VIEWER_ROLE: &str = "viewer";
const MAINTAINER_ROLE: &str = "maintainer";
const OWNER_ROLE: &str = "owner";

/// The fixed engine shared by the whole process.
#[derive(Clone)]
pub struct Context69Authorizer {
    authorizer: Authorizer,
}

impl Context69Authorizer {
    /// Build the immutable engine once at startup.
    ///
    /// Initialization failure is returned to the caller so startup aborts
    /// instead of running with a permissive fallback.
    pub async fn new() -> Result<Self, InitError> {
        let authorizer = Authorizer::new(AuthorizerConfig {
            facts_revision: FactsRevision(1),
            domains: vec![DomainId::new(SERVICE_DOMAIN)],
            roles: role_specs(),
            assignments: Vec::new(),
            global_assignments: Vec::new(),
            hierarchy: DomainHierarchy::default(),
            public_read: BTreeSet::new(),
        })
        .await?;
        Ok(Self { authorizer })
    }

    /// Whether the actual caller currently holds the administrator role.
    ///
    /// `is_admin` is the trusted current user fact; it is never inferred from a
    /// token, header, or client claim, and the administrator role carries only
    /// its enumerated control capability.
    pub fn admin_allowed(&self, user_id: i64, is_admin: bool) -> bool {
        let Some(principal) = human_principal(user_id) else {
            return false;
        };
        let roles = if is_admin {
            BTreeSet::from([RoleId::new(ADMIN_ROLE)])
        } else {
            BTreeSet::new()
        };
        let request = request(
            &principal,
            SERVICE_DOMAIN,
            ADMIN_RESOURCE,
            ADMIN_MANAGE_ACTION,
            None,
        );
        let resolved = ResolvedRoles {
            principal,
            domain: DomainId::new(SERVICE_DOMAIN),
            roles,
        };
        self.allowed(request, resolved, false)
    }

    /// Whether the caller's credential grants one exact personal-access-token
    /// scope.
    ///
    /// `token_scopes` is `None` for a browser session, which keeps the
    /// historical unrestricted behavior. A personal-access token contributes
    /// only the exact scopes it carries; an empty set authorizes nothing.
    pub fn scope_allowed(
        &self,
        user_id: i64,
        required: PersonalAccessTokenScope,
        token_scopes: Option<&BTreeSet<PersonalAccessTokenScope>>,
    ) -> bool {
        let Some(principal) = human_principal(user_id) else {
            return false;
        };
        let credential = token_scopes.map(|scopes| CredentialContext {
            principal: principal.clone(),
            audience: service_id(),
            grants: scope_attenuation(scopes),
        });
        let request = request(
            &principal,
            SERVICE_DOMAIN,
            SCOPE_RESOURCE,
            scope_action(required),
            credential,
        );
        let resolved = ResolvedRoles {
            principal,
            domain: DomainId::new(SERVICE_DOMAIN),
            roles: BTreeSet::from([RoleId::new(AUTHENTICATED_ROLE)]),
        };
        self.allowed(request, resolved, false)
    }

    /// Whether the actual caller may exercise `required` in `group_id`.
    ///
    /// The role set comes from the live SQL-resolved `current_role`, bound to
    /// the exact dynamic `group:<id>` domain. The token group range is the
    /// credential ceiling: an unrestricted token or a browser session imposes
    /// no ceiling, while a restricted token must name this exact group. The
    /// engine evaluates the role and the ceiling in one decision.
    pub fn group_action_allowed(
        &self,
        user_id: i64,
        group_id: i64,
        required: GroupAction,
        current_role: Option<MembershipRole>,
        token_scope: Option<&TokenGroupScope>,
    ) -> bool {
        if group_id <= 0 {
            return false;
        }
        let Some(principal) = human_principal(user_id) else {
            return false;
        };
        let domain = group_domain(group_id);
        let roles = current_role
            .map(|role| BTreeSet::from([RoleId::new(role_id(role))]))
            .unwrap_or_default();
        let credential = token_scope.map(|scope| CredentialContext {
            principal: principal.clone(),
            audience: service_id(),
            grants: group_attenuation(scope),
        });
        let request = request(
            &principal,
            &domain.0,
            GROUP_RESOURCE,
            required.as_action(),
            credential,
        );
        let resolved = ResolvedRoles {
            principal,
            domain,
            roles,
        };
        self.allowed(request, resolved, true)
    }

    fn allowed(
        &self,
        request: AuthzRequest,
        resolved: ResolvedRoles,
        dynamic_domain: bool,
    ) -> bool {
        let decision = if dynamic_domain {
            self.authorizer
                .authorize_with_resolved_domain(&request, &resolved)
        } else {
            self.authorizer.authorize_with_roles(&request, &resolved)
        };
        matches!(decision, Decision::Allow)
    }
}

/// The explicit group gate action. A required role maps to one of these, never
/// to a caller-supplied policy string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupAction {
    View,
    Contribute,
    Manage,
}

impl GroupAction {
    pub fn as_action(self) -> &'static str {
        match self {
            Self::View => GROUP_VIEW_ACTION,
            Self::Contribute => GROUP_CONTRIBUTE_ACTION,
            Self::Manage => GROUP_MANAGE_ACTION,
        }
    }

    pub fn for_role(role: MembershipRole) -> Self {
        match role {
            MembershipRole::Viewer => Self::View,
            MembershipRole::Maintainer => Self::Contribute,
            MembershipRole::Owner => Self::Manage,
        }
    }
}

fn human_principal(user_id: i64) -> Option<PrincipalRef> {
    if user_id <= 0 {
        return None;
    }
    Some(PrincipalRef {
        kind: PrincipalKind::Human,
        id: PrincipalId::new(user_id.to_string()),
    })
}

fn service_id() -> ServiceId {
    ServiceId::new(SERVICE_AUDIENCE)
}

fn group_domain(group_id: i64) -> DomainId {
    DomainId::new(format!("group:{group_id}"))
}

fn role_id(role: MembershipRole) -> &'static str {
    match role {
        MembershipRole::Viewer => VIEWER_ROLE,
        MembershipRole::Maintainer => MAINTAINER_ROLE,
        MembershipRole::Owner => OWNER_ROLE,
    }
}

fn scope_action(scope: PersonalAccessTokenScope) -> &'static str {
    match scope {
        PersonalAccessTokenScope::Search => "search",
        PersonalAccessTokenScope::Workspace => "workspace",
        PersonalAccessTokenScope::Library => "library",
        PersonalAccessTokenScope::Sources => "sources",
        PersonalAccessTokenScope::Settings => "settings",
        PersonalAccessTokenScope::Admin => "admin",
    }
}

fn scopes() -> [PersonalAccessTokenScope; 6] {
    [
        PersonalAccessTokenScope::Search,
        PersonalAccessTokenScope::Workspace,
        PersonalAccessTokenScope::Library,
        PersonalAccessTokenScope::Sources,
        PersonalAccessTokenScope::Settings,
        PersonalAccessTokenScope::Admin,
    ]
}

fn capability(resource: &str, action: &str) -> Capability {
    Capability {
        resource: ResourceKind::new(resource),
        action: Action::new(action),
    }
}

fn role_specs() -> Vec<RoleSpec> {
    let scope_capabilities = scopes()
        .into_iter()
        .map(|scope| capability(SCOPE_RESOURCE, scope_action(scope)))
        .collect::<BTreeSet<_>>();
    let viewer = BTreeSet::from([capability(GROUP_RESOURCE, GROUP_VIEW_ACTION)]);
    let maintainer = BTreeSet::from([
        capability(GROUP_RESOURCE, GROUP_VIEW_ACTION),
        capability(GROUP_RESOURCE, GROUP_CONTRIBUTE_ACTION),
    ]);
    let owner = BTreeSet::from([
        capability(GROUP_RESOURCE, GROUP_VIEW_ACTION),
        capability(GROUP_RESOURCE, GROUP_CONTRIBUTE_ACTION),
        capability(GROUP_RESOURCE, GROUP_MANAGE_ACTION),
    ]);
    vec![
        RoleSpec {
            id: RoleId::new(ADMIN_ROLE),
            capabilities: BTreeSet::from([capability(ADMIN_RESOURCE, ADMIN_MANAGE_ACTION)]),
        },
        RoleSpec {
            id: RoleId::new(AUTHENTICATED_ROLE),
            capabilities: scope_capabilities,
        },
        RoleSpec {
            id: RoleId::new(VIEWER_ROLE),
            capabilities: viewer,
        },
        RoleSpec {
            id: RoleId::new(MAINTAINER_ROLE),
            capabilities: maintainer,
        },
        RoleSpec {
            id: RoleId::new(OWNER_ROLE),
            capabilities: owner,
        },
    ]
}

fn scope_attenuation(scopes: &BTreeSet<PersonalAccessTokenScope>) -> Attenuation {
    let grants = scopes
        .iter()
        .map(|scope| Grant {
            domain: DomainId::new(SERVICE_DOMAIN),
            resource: ResourceKind::new(SCOPE_RESOURCE),
            resource_id: None,
            action: Action::new(scope_action(*scope)),
        })
        .collect::<BTreeSet<_>>();
    Attenuation::Grants(grants)
}

fn group_attenuation(scope: &TokenGroupScope) -> Attenuation {
    let Some(group_ids) = scope.restricted_group_ids() else {
        return Attenuation::Unrestricted;
    };
    let grants = group_ids
        .iter()
        .flat_map(|group_id| {
            let domain = group_domain(*group_id);
            [
                GroupAction::View,
                GroupAction::Contribute,
                GroupAction::Manage,
            ]
            .into_iter()
            .map(move |action| Grant {
                domain: domain.clone(),
                resource: ResourceKind::new(GROUP_RESOURCE),
                resource_id: None,
                action: Action::new(action.as_action()),
            })
        })
        .collect::<BTreeSet<_>>();
    Attenuation::Grants(grants)
}

fn request(
    principal: &PrincipalRef,
    domain: &str,
    resource: &str,
    action: &str,
    credential: Option<CredentialContext>,
) -> AuthzRequest {
    AuthzRequest {
        subject: Subject::Principal(principal.clone()),
        domain: DomainId::new(domain),
        resource: ResourceRef {
            kind: ResourceKind::new(resource),
            id: None,
            domain: DomainId::new(domain),
        },
        action: Action::new(action),
        audience: service_id(),
        facts_revision: FactsRevision(1),
        credential,
        server: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use context69_contracts_auth::PersonalAccessTokenScope;
    use context69_contracts_core::TokenGroupScope;
    use context69_contracts_namespace::MembershipRole;

    use super::{Context69Authorizer, GroupAction};

    fn engine() -> Context69Authorizer {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(Context69Authorizer::new())
            .expect("engine")
    }

    #[test]
    fn administrator_role_requires_the_current_admin_fact() {
        let engine = engine();
        assert!(engine.admin_allowed(7, true));
        assert!(!engine.admin_allowed(7, false));
        assert!(!engine.admin_allowed(0, true));
    }

    #[test]
    fn personal_access_token_scopes_are_exact_and_empty_denies() {
        let engine = engine();
        let search = BTreeSet::from([PersonalAccessTokenScope::Search]);
        assert!(engine.scope_allowed(7, PersonalAccessTokenScope::Search, Some(&search)));
        assert!(!engine.scope_allowed(7, PersonalAccessTokenScope::Admin, Some(&search)));
        assert!(
            !engine.scope_allowed(7, PersonalAccessTokenScope::Search, Some(&BTreeSet::new()),)
        );
        assert!(engine.scope_allowed(7, PersonalAccessTokenScope::Admin, None));
    }

    #[test]
    fn group_role_thresholds_match_the_current_matrix() {
        let engine = engine();
        for (role, view, contribute, manage) in [
            (Some(MembershipRole::Viewer), true, false, false),
            (Some(MembershipRole::Maintainer), true, true, false),
            (Some(MembershipRole::Owner), true, true, true),
            (None, false, false, false),
        ] {
            assert_eq!(
                engine.group_action_allowed(7, 3, GroupAction::View, role, None),
                view
            );
            assert_eq!(
                engine.group_action_allowed(7, 3, GroupAction::Contribute, role, None),
                contribute
            );
            assert_eq!(
                engine.group_action_allowed(7, 3, GroupAction::Manage, role, None),
                manage
            );
        }
    }

    #[test]
    fn every_required_role_maps_to_its_explicit_gate_action() {
        assert_eq!(
            GroupAction::for_role(MembershipRole::Viewer),
            GroupAction::View
        );
        assert_eq!(
            GroupAction::for_role(MembershipRole::Maintainer),
            GroupAction::Contribute
        );
        assert_eq!(
            GroupAction::for_role(MembershipRole::Owner),
            GroupAction::Manage
        );
    }

    #[test]
    fn a_restricted_token_ceiling_cannot_cross_group_boundaries() {
        let engine = engine();
        let scope = TokenGroupScope::Groups { group_ids: vec![3] };
        assert!(engine.group_action_allowed(
            7,
            3,
            GroupAction::View,
            Some(MembershipRole::Owner),
            Some(&scope),
        ));
        assert!(!engine.group_action_allowed(
            7,
            4,
            GroupAction::View,
            Some(MembershipRole::Owner),
            Some(&scope),
        ));
        assert!(!engine.group_action_allowed(
            7,
            3,
            GroupAction::View,
            Some(MembershipRole::Owner),
            Some(&TokenGroupScope::Groups {
                group_ids: Vec::new()
            }),
        ));
    }

    #[test]
    fn unrestricted_ceiling_still_requires_a_live_role() {
        let engine = engine();
        assert!(engine.group_action_allowed(
            7,
            9,
            GroupAction::View,
            Some(MembershipRole::Viewer),
            None
        ));
        assert!(!engine.group_action_allowed(7, 9, GroupAction::View, None, None));
    }

    #[test]
    fn a_dynamic_group_works_without_registration() {
        let engine = engine();
        assert!(engine.group_action_allowed(
            7,
            987_654,
            GroupAction::View,
            Some(MembershipRole::Viewer),
            Some(&TokenGroupScope::All),
        ));
    }
}
