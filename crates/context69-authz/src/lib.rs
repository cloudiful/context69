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
    Action, Attenuation, Authority, Authorizer, AuthorizerConfig, AuthzRequest, Capability,
    CredentialContext, Decision, DomainHierarchy, DomainId, FactsRevision, Grant, PrincipalId,
    PrincipalKind, PrincipalRef, ResolvedRoles, ResourceKind, ResourceRef, RoleId, RoleSpec,
    ServiceId,
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

/// A decision using trusted current facts resolved by the caller.
pub enum Operation<'a> {
    Admin {
        is_admin: bool,
    },
    Scope {
        required: PersonalAccessTokenScope,
        token_scopes: Option<&'a BTreeSet<PersonalAccessTokenScope>>,
    },
    Group {
        group_id: i64,
        required: GroupAction,
        current_role: Option<MembershipRole>,
        token_scope: Option<&'a TokenGroupScope>,
    },
}

enum DomainAuthority {
    Registered,
    Resolved,
}

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

    /// Evaluate an operation against trusted current user, role and credential facts.
    pub fn check(&self, user_id: i64, operation: Operation<'_>) -> bool {
        let Some(principal) = human_principal(user_id) else {
            return false;
        };
        let (domain, roles, resource, action, grants, domain_authority) = match operation {
            Operation::Admin { is_admin } => (
                DomainId::new(SERVICE_DOMAIN),
                if is_admin {
                    BTreeSet::from([RoleId::new(ADMIN_ROLE)])
                } else {
                    BTreeSet::new()
                },
                ADMIN_RESOURCE,
                ADMIN_MANAGE_ACTION,
                None,
                DomainAuthority::Registered,
            ),
            Operation::Scope {
                required,
                token_scopes,
            } => (
                DomainId::new(SERVICE_DOMAIN),
                BTreeSet::from([RoleId::new(AUTHENTICATED_ROLE)]),
                SCOPE_RESOURCE,
                scope_action(required),
                token_scopes.map(scope_attenuation),
                DomainAuthority::Registered,
            ),
            Operation::Group {
                group_id,
                required,
                current_role,
                token_scope,
            } => {
                if group_id <= 0 {
                    return false;
                }
                (
                    group_domain(group_id),
                    current_role
                        .map(|role| BTreeSet::from([RoleId::new(role_id(role))]))
                        .unwrap_or_default(),
                    GROUP_RESOURCE,
                    required.as_action(),
                    token_scope.map(group_attenuation),
                    DomainAuthority::Resolved,
                )
            }
        };
        let credential = grants.map(|grants| CredentialContext {
            principal: principal.clone(),
            audience: service_id(),
            grants,
        });
        let request = request(&domain.0, resource, action, credential);
        let resolved = ResolvedRoles::new(principal, domain, roles);
        let authority = match domain_authority {
            DomainAuthority::Registered => Authority::Registered(&resolved),
            DomainAuthority::Resolved => Authority::Resolved(&resolved),
        };
        matches!(
            self.authorizer.authorize(&request, authority),
            Decision::Allow
        )
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
    domain: &str,
    resource: &str,
    action: &str,
    credential: Option<CredentialContext>,
) -> AuthzRequest {
    AuthzRequest {
        resource: ResourceRef {
            kind: ResourceKind::new(resource),
            id: None,
            domain: DomainId::new(domain),
        },
        action: Action::new(action),
        audience: service_id(),
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
        assert!(engine.check(7, crate::Operation::Admin { is_admin: true }));
        assert!(!engine.check(7, crate::Operation::Admin { is_admin: false }));
        assert!(!engine.check(0, crate::Operation::Admin { is_admin: true }));
    }

    #[test]
    fn personal_access_token_scopes_are_exact_and_empty_denies() {
        let engine = engine();
        let search = BTreeSet::from([PersonalAccessTokenScope::Search]);
        assert!(engine.check(
            7,
            crate::Operation::Scope {
                required: PersonalAccessTokenScope::Search,
                token_scopes: Some(&search)
            }
        ));
        assert!(!engine.check(
            7,
            crate::Operation::Scope {
                required: PersonalAccessTokenScope::Admin,
                token_scopes: Some(&search)
            }
        ));
        assert!(!engine.check(
            7,
            crate::Operation::Scope {
                required: PersonalAccessTokenScope::Search,
                token_scopes: Some(&BTreeSet::new())
            }
        ));
        assert!(engine.check(
            7,
            crate::Operation::Scope {
                required: PersonalAccessTokenScope::Admin,
                token_scopes: None
            }
        ));
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
                engine.check(
                    7,
                    crate::Operation::Group {
                        group_id: 3,
                        required: GroupAction::View,
                        current_role: role,
                        token_scope: None
                    }
                ),
                view
            );
            assert_eq!(
                engine.check(
                    7,
                    crate::Operation::Group {
                        group_id: 3,
                        required: GroupAction::Contribute,
                        current_role: role,
                        token_scope: None
                    }
                ),
                contribute
            );
            assert_eq!(
                engine.check(
                    7,
                    crate::Operation::Group {
                        group_id: 3,
                        required: GroupAction::Manage,
                        current_role: role,
                        token_scope: None
                    }
                ),
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
        assert!(engine.check(
            7,
            crate::Operation::Group {
                group_id: 3,
                required: GroupAction::View,
                current_role: Some(MembershipRole::Owner),
                token_scope: Some(&scope)
            }
        ));
        assert!(!engine.check(
            7,
            crate::Operation::Group {
                group_id: 4,
                required: GroupAction::View,
                current_role: Some(MembershipRole::Owner),
                token_scope: Some(&scope)
            }
        ));
        assert!(!engine.check(
            7,
            crate::Operation::Group {
                group_id: 3,
                required: GroupAction::View,
                current_role: Some(MembershipRole::Owner),
                token_scope: Some(&TokenGroupScope::Groups {
                    group_ids: Vec::new()
                })
            }
        ));
    }

    #[test]
    fn unrestricted_ceiling_still_requires_a_live_role() {
        let engine = engine();
        assert!(engine.check(
            7,
            crate::Operation::Group {
                group_id: 9,
                required: GroupAction::View,
                current_role: Some(MembershipRole::Viewer),
                token_scope: None
            }
        ));
        assert!(!engine.check(
            7,
            crate::Operation::Group {
                group_id: 9,
                required: GroupAction::View,
                current_role: None,
                token_scope: None
            }
        ));
    }

    #[test]
    fn a_dynamic_group_works_without_registration() {
        let engine = engine();
        assert!(engine.check(
            7,
            crate::Operation::Group {
                group_id: 987_654,
                required: GroupAction::View,
                current_role: Some(MembershipRole::Viewer),
                token_scope: Some(&TokenGroupScope::All)
            }
        ));
    }
}
