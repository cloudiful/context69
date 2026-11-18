use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The group range a personal access token is limited to.
///
/// `All` follows the owner's current accessible groups. `Groups` names the
/// exact stable group ids the token may use; it never grows implicitly and is
/// still intersected with the owner's current access at request time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TokenGroupScope {
    All,
    Groups { group_ids: Vec<i64> },
}

impl TokenGroupScope {
    /// The restricted group ids, or `None` for the unrestricted mode.
    pub fn restricted_group_ids(&self) -> Option<&[i64]> {
        match self {
            Self::All => None,
            Self::Groups { group_ids } => Some(group_ids),
        }
    }

    /// The explicit ids of the restricted mode, normalized for persistence.
    pub fn normalized_group_ids(&self) -> Option<Vec<i64>> {
        self.restricted_group_ids().map(|ids| {
            let mut ids = ids.to_vec();
            ids.sort_unstable();
            ids.dedup();
            ids
        })
    }

    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }

    /// Whether this range is the exact `Groups` mode rather than `All`.
    pub fn is_restricted(&self) -> bool {
        matches!(self, Self::Groups { .. })
    }

    /// Whether this range names `group_id`.
    pub fn allows(&self, group_id: i64) -> bool {
        match self {
            Self::All => true,
            Self::Groups { group_ids } => group_ids.contains(&group_id),
        }
    }
}

/// How the recorded last-used instant must be interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PersonalAccessTokenLastUsedPrecision {
    /// The instant of the most recent successful authorized use observed since
    /// the token's usage tracking switched to precise timestamps.
    Exact,
    /// Legacy telemetry: the first observation of the last UTC day the token
    /// was used, not an exact last-use instant.
    LegacyDay,
}

#[cfg(test)]
mod tests {
    use super::TokenGroupScope;

    #[test]
    fn all_allows_every_group_and_normalizes_to_no_ids() {
        let scope = TokenGroupScope::All;
        assert!(scope.is_all());
        assert!(!scope.is_restricted());
        assert!(scope.allows(42));
        assert!(scope.restricted_group_ids().is_none());
        assert!(scope.normalized_group_ids().is_none());
    }

    #[test]
    fn a_restricted_scope_allows_only_its_normalized_ids() {
        let scope = TokenGroupScope::Groups {
            group_ids: vec![9, 3, 9],
        };
        assert!(!scope.is_all());
        assert!(scope.is_restricted());
        assert!(scope.allows(3));
        assert!(scope.allows(9));
        assert!(!scope.allows(4));
        assert_eq!(scope.normalized_group_ids(), Some(vec![3, 9]));
    }

    #[test]
    fn the_wire_shape_separates_all_from_an_explicit_set() {
        let all = serde_json::to_value(TokenGroupScope::All).expect("serialize");
        assert_eq!(all, serde_json::json!({ "mode": "all" }));

        let groups = serde_json::to_value(TokenGroupScope::Groups {
            group_ids: vec![3, 9],
        })
        .expect("serialize");
        assert_eq!(
            groups,
            serde_json::json!({ "mode": "groups", "group_ids": [3, 9] })
        );
    }
}
