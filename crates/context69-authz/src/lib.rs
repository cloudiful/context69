use std::collections::BTreeSet;

use casbin::{CoreApi, DefaultModel, Enforcer, MemoryAdapter, MgmtApi};
use context69_contracts_auth::PersonalAccessTokenScope;
use context69_contracts_namespace::MembershipRole;

const MODEL: &str = r#"
[request_definition]
r = sub, obj, act
[policy_definition]
p = sub, obj, act
[policy_effect]
e = some(where (p.eft == allow))
[matchers]
m = r.sub == p.sub && r.obj == p.obj && r.act == p.act
"#;

static ENGINE: tokio::sync::OnceCell<Result<Enforcer, &'static str>> =
    tokio::sync::OnceCell::const_new();

async fn build_engine() -> Result<Enforcer, &'static str> {
    let model = DefaultModel::from_str(MODEL)
        .await
        .map_err(|_| "authorization initialization failed")?;
    let mut enforcer = Enforcer::new(model, MemoryAdapter::default())
        .await
        .map_err(|_| "authorization initialization failed")?;
    let policies = [
        ("admin", "global", "admin"),
        ("owner", "group", "owner"),
        ("owner", "group", "maintainer"),
        ("owner", "group", "viewer"),
        ("maintainer", "group", "maintainer"),
        ("maintainer", "group", "viewer"),
        ("viewer", "group", "viewer"),
        ("search", "scope", "search"),
        ("workspace", "scope", "workspace"),
        ("library", "scope", "library"),
        ("sources", "scope", "sources"),
        ("settings", "scope", "settings"),
        ("admin", "scope", "admin"),
    ]
    .into_iter()
    .map(|(s, o, a)| vec![s.to_owned(), o.to_owned(), a.to_owned()])
    .collect();
    enforcer
        .add_policies(policies)
        .await
        .map_err(|_| "authorization initialization failed")?;
    Ok(enforcer)
}

pub async fn initialize() -> Result<(), &'static str> {
    ENGINE
        .get_or_init(build_engine)
        .await
        .as_ref()
        .map(|_| ())
        .map_err(|_| "authorization initialization failed")
}

fn engine() -> Result<&'static Enforcer, &'static str> {
    // Synchronous callers outside app startup cannot safely nest an async executor.
    if ENGINE.get().is_none() {
        std::thread::Builder::new()
            .name("context69-authz-init".into())
            .spawn(|| futures::executor::block_on(initialize()))
            .map_err(|_| "authorization initialization failed")?
            .join()
            .map_err(|_| "authorization initialization failed")??;
    }
    ENGINE
        .get()
        .and_then(|result| result.as_ref().ok())
        .ok_or("authorization initialization failed")
}

fn decide(subject: &str, object: &str, action: &str) -> bool {
    engine()
        .and_then(|e| {
            e.enforce((subject, object, action))
                .map_err(|_| "authorization enforcement failed")
        })
        .unwrap_or(false)
}

pub fn admin(is_admin: bool) -> bool {
    decide(if is_admin { "admin" } else { "user" }, "global", "admin")
}

pub fn group(actual: Option<MembershipRole>, required: MembershipRole) -> bool {
    decide(
        actual.map(MembershipRole::as_str).unwrap_or("none"),
        "group",
        required.as_str(),
    )
}

fn scope_name(scope: PersonalAccessTokenScope) -> &'static str {
    match scope {
        PersonalAccessTokenScope::Search => "search",
        PersonalAccessTokenScope::Workspace => "workspace",
        PersonalAccessTokenScope::Library => "library",
        PersonalAccessTokenScope::Sources => "sources",
        PersonalAccessTokenScope::Settings => "settings",
        PersonalAccessTokenScope::Admin => "admin",
    }
}

pub fn scope(
    scopes: &BTreeSet<PersonalAccessTokenScope>,
    required: PersonalAccessTokenScope,
) -> bool {
    scopes
        .iter()
        .any(|scope| decide(scope_name(*scope), "scope", scope_name(required)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policies_enforce_boundaries() {
        assert!(futures::executor::block_on(initialize()).is_ok());
        assert!(admin(true));
        assert!(!admin(false));
        for (actual, expected) in [
            (None, false),
            (Some(MembershipRole::Viewer), false),
            (Some(MembershipRole::Maintainer), true),
            (Some(MembershipRole::Owner), true),
        ] {
            assert_eq!(group(actual, MembershipRole::Maintainer), expected);
        }
        let roles = [
            MembershipRole::Viewer,
            MembershipRole::Maintainer,
            MembershipRole::Owner,
        ];
        for (i, actual) in roles.iter().enumerate() {
            for (j, required) in roles.iter().enumerate() {
                assert_eq!(group(Some(*actual), *required), i >= j);
            }
        }
        let all = [
            PersonalAccessTokenScope::Search,
            PersonalAccessTokenScope::Workspace,
            PersonalAccessTokenScope::Library,
            PersonalAccessTokenScope::Sources,
            PersonalAccessTokenScope::Settings,
            PersonalAccessTokenScope::Admin,
        ];
        for granted in all {
            for required in all {
                assert_eq!(
                    scope(&BTreeSet::from([granted]), required),
                    granted == required
                );
            }
        }
        assert!(!decide("admin", "scope", "unknown"));
    }
}
