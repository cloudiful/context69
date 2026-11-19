use std::time::Duration;

use anyhow::{Error, Result};
use context69_contracts::{
    CreateGroupRequest, CreateMetadataIndexRequest, EnsureScopeResponse, MembershipRole,
    MetadataIndexResponse, MetadataIndexStatus, ScopeSpec, TokenGroupScope,
};
use tokio::time::sleep;

use crate::{
    domain::GroupRecord, domain_errors::DomainError, services::document_store::DocumentStoreService,
};

use super::{TaskService, responses::group_response};

impl TaskService {
    pub async fn ensure_scope(
        &self,
        user_id: i64,
        token_group_scope: Option<&TokenGroupScope>,
        spec: &ScopeSpec,
    ) -> Result<EnsureScopeResponse> {
        let actor = self
            .db
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("user not found"))?;
        let (parent_group_path, group_key) = split_scope_path(&spec.group_path)?;
        let group = match self
            .namespace
            .get_group_for_user(user_id, &spec.group_path, None)
            .await?
        {
            Some(group) => group,
            None => match self
                .namespace
                .create_group(
                    &actor,
                    &CreateGroupRequest {
                        parent_group_path,
                        group_key,
                        name: spec.name.clone(),
                        visibility: spec.visibility,
                        kind: spec.kind,
                    },
                )
                .await
            {
                Ok(group) => group,
                Err(error) if is_conflict_error(&error) => self
                    .namespace
                    .get_group_for_user(user_id, &spec.group_path, None)
                    .await?
                    .ok_or_else(|| DomainError::conflict("scope creation conflicted"))?,
                Err(error) => return Err(error),
            },
        };
        ensure_group_definition(&group, spec)?;
        let mut indexes = Vec::new();
        for requested in &spec.metadata_indexes {
            let existing = self
                .document_store
                .list_indexes(group.id, &requested.source_key)
                .await?
                .into_iter()
                .find(|index| index.path == requested.definition.path);
            match existing {
                Some(index) => {
                    ensure_index_definition(&index, &requested.definition)?;
                    index
                }
                None => {
                    require_index_write_role(&self.authorizer, user_id, &group, token_group_scope)?;
                    match self
                        .document_store
                        .create_index(
                            group.id,
                            &group.group_path,
                            &requested.source_key,
                            &requested.definition,
                        )
                        .await
                    {
                        Ok(index) => index,
                        Err(error) if is_conflict_error(&error) => self
                            .document_store
                            .list_indexes(group.id, &requested.source_key)
                            .await?
                            .into_iter()
                            .find(|index| index.path == requested.definition.path)
                            .ok_or_else(|| {
                                DomainError::conflict("metadata index creation conflicted")
                            })?,
                        Err(error) => return Err(error),
                    }
                }
            };
            indexes.push(
                wait_for_index(
                    &self.document_store,
                    group.id,
                    &requested.source_key,
                    &requested.definition.path,
                )
                .await?,
            );
        }
        Ok(EnsureScopeResponse {
            group: group_response(group),
            metadata_indexes: indexes,
        })
    }
}

fn split_scope_path(path: &str) -> Result<(Option<String>, String)> {
    let mut parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key = parts
        .pop()
        .ok_or_else(|| DomainError::invalid_argument("scope group_path must not be empty"))?;
    if parts.iter().any(|part| part.len() > 100) || key.len() > 100 {
        return Err(DomainError::invalid_argument("scope path segment is too long").into());
    }
    Ok((
        (!parts.is_empty()).then(|| parts.join("/")),
        key.to_string(),
    ))
}

fn ensure_group_definition(group: &GroupRecord, spec: &ScopeSpec) -> Result<()> {
    if group.name != spec.name
        || group.visibility != spec.visibility
        || spec.kind.is_some_and(|kind| group.kind != kind)
    {
        return Err(DomainError::conflict(format!(
            "scope definition conflict for {}",
            spec.group_path
        ))
        .into());
    }
    Ok(())
}

/// Creating an index changes an existing group's write state, so the owner's
/// current role must satisfy the same Maintainer rule as the normal API path.
/// A newly bootstrapped group returns the creator as Owner and therefore passes.
fn require_index_write_role(
    authorizer: &context69_authz::Context69Authorizer,
    user_id: i64,
    group: &GroupRecord,
    token_group_scope: Option<&TokenGroupScope>,
) -> Result<()> {
    if authorizer.check(
        user_id,
        context69_authz::Operation::Group {
            group_id: group.id,
            required: context69_authz::GroupAction::for_role(MembershipRole::Maintainer),
            current_role: group.current_role,
            token_scope: token_group_scope,
        },
    ) {
        Ok(())
    } else {
        Err(DomainError::forbidden("insufficient permissions for group").into())
    }
}

fn ensure_index_definition(
    index: &MetadataIndexResponse,
    definition: &CreateMetadataIndexRequest,
) -> Result<()> {
    if index.data_type != definition.data_type
        || index.value_kind != definition.value_kind
        || index.sortable != definition.sortable
    {
        return Err(DomainError::conflict(format!(
            "metadata index definition conflict for {}",
            definition.path
        ))
        .into());
    }
    Ok(())
}

async fn wait_for_index(
    service: &DocumentStoreService,
    group_id: i64,
    source_key: &str,
    path: &str,
) -> Result<MetadataIndexResponse> {
    for _ in 0..120 {
        if let Some(index) = service
            .list_indexes(group_id, source_key)
            .await?
            .into_iter()
            .find(|index| index.path == path)
        {
            match index.status {
                MetadataIndexStatus::Ready => return Ok(index),
                MetadataIndexStatus::Failed => {
                    return Err(DomainError::internal(
                        index
                            .error_message
                            .unwrap_or_else(|| "metadata index build failed".to_string()),
                    )
                    .into());
                }
                _ => sleep(Duration::from_millis(50)).await,
            }
        } else {
            sleep(Duration::from_millis(50)).await;
        }
    }
    Err(
        DomainError::upstream_timeout(format!("timed out waiting for metadata index {path}"))
            .into(),
    )
}

fn is_conflict_error(error: &Error) -> bool {
    if matches!(
        crate::domain_errors::find_domain_error(error),
        Some(crate::domain_errors::DomainError::Conflict(_))
    ) {
        return true;
    }
    // Postgres unique-violation without message sniffing: SQLSTATE 23505.
    for cause in error.chain() {
        if let Some(db_error) = cause.downcast_ref::<sqlx::Error>()
            && let sqlx::Error::Database(database_error) = db_error
            && database_error.code().as_deref() == Some("23505")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::split_scope_path;

    #[test]
    fn scope_path_is_split_without_empty_segments() {
        assert_eq!(
            split_scope_path("research/news").expect("scope"),
            (Some("research".into()), "news".into())
        );
        assert_eq!(
            split_scope_path("/news/").expect("scope"),
            (None, "news".into())
        );
        assert!(split_scope_path("/").is_err());
    }
}

#[cfg(test)]
mod role_tests {
    use chrono::Utc;

    use super::require_index_write_role;
    use crate::{
        contracts::{GroupKind, MembershipRole, Visibility},
        domain::GroupRecord,
    };

    fn authorizer() -> context69_authz::Context69Authorizer {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(context69_authz::Context69Authorizer::new())
            .expect("engine")
    }

    fn check(group: &GroupRecord) -> anyhow::Result<()> {
        require_index_write_role(&authorizer(), 2, group, None)
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
            owner_user_id: Some(2),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            current_role,
        }
    }

    #[test]
    fn existing_group_index_creation_requires_maintainer_or_owner() {
        assert!(check(&group(None)).is_err());
        assert!(check(&group(Some(MembershipRole::Viewer))).is_err());
        assert!(check(&group(Some(MembershipRole::Maintainer))).is_ok());
        assert!(check(&group(Some(MembershipRole::Owner))).is_ok());
    }

    #[test]
    fn a_newly_bootstrapped_group_owner_can_create_its_index() {
        assert!(check(&group(Some(MembershipRole::Owner))).is_ok());
    }
}
