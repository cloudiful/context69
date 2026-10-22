//! The domain adapter for the admin Docling activity list.
//!
//! The protocol lives in [`crate::docling::client`]: the bounded list, the
//! queued cancel, and the immediate status read. This module only resolves
//! the saved endpoint and maps the client results onto the admin contract, so
//! the admin DTO keeps its fields, its counts, and its bounded errors.

use crate::contracts::{
    DOCLING_REMOTE_LIST_LIMIT_MAX, DOCLING_REMOTE_LIST_LIMIT_MIN, DoclingRemoteTask,
    DoclingRemoteTasksResponse,
};
use crate::docling::DoclingClient;
use crate::docling::client::REMOTE_OP_TIMEOUT;
use crate::docling::error::DoclingError;
use crate::domain_errors::DomainError;

fn validate_limit(limit: u32) -> anyhow::Result<()> {
    if !(DOCLING_REMOTE_LIST_LIMIT_MIN..=DOCLING_REMOTE_LIST_LIMIT_MAX).contains(&limit) {
        return Err(DomainError::invalid_argument(format!(
            "limit must be between 1 and 100, got {limit}"
        ))
        .into());
    }
    Ok(())
}

/// Maps one client failure onto the bounded admin error.
///
/// The endpoint, the query, and the remote body never reach the message.
fn list_error(error: DoclingError) -> anyhow::Error {
    match error {
        DoclingError::Http { status } => match status {
            401 | 403 => {
                DomainError::upstream_error("docling remote requires authentication").into()
            }
            404 => {
                DomainError::upstream_error("docling remote list is not supported by this endpoint")
                    .into()
            }
            429 => DomainError::unavailable("docling remote is temporarily unavailable").into(),
            500..=599 => {
                DomainError::unavailable("docling remote is temporarily unavailable").into()
            }
            _ => DomainError::upstream_error("docling remote list failed").into(),
        },
        DoclingError::Timeout => {
            DomainError::upstream_timeout("docling remote list timed out").into()
        }
        DoclingError::Transport { .. } => {
            DomainError::unavailable("docling remote is temporarily unavailable").into()
        }
        DoclingError::Parse { .. } | DoclingError::IdentityMismatch => {
            DomainError::upstream_error("docling remote returned a malformed task list").into()
        }
        DoclingError::Validation { parameter, reason } => {
            DomainError::invalid_argument(format!("docling.{parameter}: {reason}")).into()
        }
        DoclingError::Configuration { .. }
        | DoclingError::ResultTooLarge { .. }
        | DoclingError::TaskFailed { .. } => {
            DomainError::upstream_error("docling remote list failed").into()
        }
    }
}

impl super::LibraryService {
    /// List one bounded page of remote Docling task identities.
    ///
    /// Uses the saved endpoint only. `limit` is `1..=100`, `offset` is `>=0`;
    /// the remote's `has_more` is returned verbatim and no total is invented.
    /// Unknown `task_status` values stay text and are returned as-is.
    pub(crate) async fn list_docling_remote_tasks(
        &self,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<DoclingRemoteTasksResponse> {
        validate_limit(limit)?;
        let config = self
            .settings
            .resolve_docling_config()
            .await?
            .ok_or_else(|| DomainError::internal("docling is not configured"))?;
        let client = DoclingClient::with_timeout(&config.connection.base_url, REMOTE_OP_TIMEOUT)?;
        let page = client.list_tasks(limit, offset).await.map_err(list_error)?;
        Ok(DoclingRemoteTasksResponse {
            items: page
                .items
                .into_iter()
                .map(|item| DoclingRemoteTask {
                    task_id: item.task_id,
                    task_type: item.task_type,
                    task_status: item.task_status,
                    created_at: item.created_at,
                })
                .collect(),
            has_more: page.has_more,
            cleanup: Vec::new(),
            remote_error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn limit_validation_matches_the_fork_bounds() {
        assert!(super::validate_limit(0).is_err());
        assert!(super::validate_limit(1).is_ok());
        assert!(super::validate_limit(20).is_ok());
        assert!(super::validate_limit(100).is_ok());
        assert!(super::validate_limit(101).is_err());
    }
}
