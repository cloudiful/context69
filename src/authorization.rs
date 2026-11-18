//! Request-scoped group authorization for personal access tokens.
//!
//! A restricted token may use only the exact group ids it names, intersected
//! with the owner's current access. Unrestricted tokens and browser sessions
//! keep the historical behavior.

use anyhow::Result;

use crate::contracts::TokenGroupScope;
use crate::domain_errors::DomainError;

/// Whether a request carrying `scope` may use `group_id`.
pub fn group_allowed(scope: Option<&TokenGroupScope>, group_id: i64) -> bool {
    scope.is_none_or(|scope| scope.allows(group_id))
}

/// Refuse a group that the token's group range does not name.
pub fn require_group_allowed(scope: Option<&TokenGroupScope>, group_id: i64) -> Result<()> {
    if group_allowed(scope, group_id) {
        Ok(())
    } else {
        Err(DomainError::forbidden("insufficient permissions for group").into())
    }
}

/// Refuse a global operation that cannot be constrained to a group range.
///
/// A group-restricted token is never allowed to fall back to a global listing,
/// directory, or configuration surface.
pub fn require_unrestricted(scope: Option<&TokenGroupScope>) -> Result<()> {
    match scope {
        None | Some(TokenGroupScope::All) => Ok(()),
        Some(TokenGroupScope::Groups { .. }) => Err(DomainError::forbidden(
            "this endpoint is not available to a group-restricted personal access token",
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{group_allowed, require_group_allowed, require_unrestricted};
    use crate::contracts::TokenGroupScope;

    #[test]
    fn unrestricted_scopes_allow_every_group() {
        assert!(group_allowed(None, 7));
        assert!(group_allowed(Some(&TokenGroupScope::All), 7));
        assert!(require_unrestricted(Some(&TokenGroupScope::All)).is_ok());
    }

    #[test]
    fn restricted_scopes_allow_only_named_groups() {
        let scope = TokenGroupScope::Groups {
            group_ids: vec![3, 9],
        };
        assert!(group_allowed(Some(&scope), 3));
        assert!(group_allowed(Some(&scope), 9));
        assert!(!group_allowed(Some(&scope), 4));
        assert!(require_group_allowed(Some(&scope), 4).is_err());
        assert!(require_unrestricted(Some(&scope)).is_err());
    }

    #[test]
    fn an_empty_restricted_set_allows_nothing() {
        let scope = TokenGroupScope::Groups { group_ids: vec![] };
        assert!(!group_allowed(Some(&scope), 1));
    }
}
