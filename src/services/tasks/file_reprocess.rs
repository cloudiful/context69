//! File-level batch reprocessing admission (issue 83 P1).
//!
//! A succeeded library file can be processed again from its current source:
//! the retained original bytes when they are still there, otherwise the
//! original HTTP(S) source. The admission is a fresh `rerun_source` fork of
//! the named files, so the worker runs the durable replacement engine and the
//! previous committed result stays until the replacement switches atomically.
//!
//! Every requested id gets its own result in request order, but the files are
//! admitted as one task: the transaction takes the same sorted per-file
//! advisory locks as create/retry/rerun before it reads the authoritative
//! release and status facts, so a concurrent source release cannot race the
//! admission and two admissions cannot process the same file.

use std::collections::HashMap;

use anyhow::Result;
use context69_contracts::{
    LibraryFileReprocessOutcome, LibraryFileReprocessReason, LibraryFileReprocessResult,
    MAX_REPROCESS_FILE_IDS, ReprocessLibraryFilesResponse,
};
use uuid::Uuid;

use super::TaskService;
use crate::db::{NewFileReprocessItem, active_file_task_items, lock_file_processing_slots};
use crate::domain_errors::DomainError;
use crate::services::library::{can_reprocess_source, has_downloadable_source};

/// Payload marker that lets a `file_batch` rerun read the retained source
/// first and fall back to the original URL when the stored bytes are gone.
pub(crate) const SOURCE_FALLBACK_PAYLOAD_KEY: &str = "source_fallback";
/// The only fallback value understood by the worker.
pub(crate) const SOURCE_FALLBACK_URL: &str = "url";

/// One requested batch of file ids.
pub(crate) struct FileReprocessRequest<'a> {
    pub user_id: i64,
    pub group_id: i64,
    pub group_path: &'a str,
    pub file_ids: &'a [Uuid],
}

/// One requested file and the decision the admission reached for it.
///
/// `Accepted` is never stored here: an accepted file is simply absent from the
/// decision map. A decision therefore always carries its terminal outcome.
struct ItemDecision {
    outcome: LibraryFileReprocessOutcome,
    reason: LibraryFileReprocessReason,
}

impl ItemDecision {
    fn skipped(reason: LibraryFileReprocessReason) -> Self {
        Self {
            outcome: LibraryFileReprocessOutcome::Skipped,
            reason,
        }
    }

    fn failed(reason: LibraryFileReprocessReason) -> Self {
        Self {
            outcome: LibraryFileReprocessOutcome::Failed,
            reason,
        }
    }
}

impl TaskService {
    /// Admit the requested files and create one reprocessing task.
    pub(crate) async fn reprocess_library_files(
        &self,
        request: FileReprocessRequest<'_>,
    ) -> Result<ReprocessLibraryFilesResponse> {
        validate_request(request.file_ids)?;

        // Durable, page-wide facts. The read also resolves ownership: an id
        // without a row in this group is reported exactly like a missing one.
        // Source values are read again under the row lock below so a concurrent
        // metadata update cannot leave this decision on stale facts.
        let facts = self
            .library()
            .source_facts_for_group(request.group_id, request.file_ids)
            .await?
            .into_iter()
            .map(|facts| (facts.id, facts))
            .collect::<HashMap<_, _>>();
        let owned = request
            .file_ids
            .iter()
            .copied()
            .filter(|file_id| facts.contains_key(file_id))
            .collect::<Vec<_>>();

        let budget = self.chunking().snapshot();
        let mut transaction = self.db().pool().begin().await?;
        // The canonical lock order serializes the admission with every other
        // path that can hand these files to a task and with source release.
        lock_file_processing_slots(&mut transaction, &owned).await?;

        let mut decisions = HashMap::new();
        for file_id in request.file_ids {
            if !facts.contains_key(file_id) {
                // Missing and foreign ids are indistinguishable on purpose.
                decisions.insert(
                    *file_id,
                    ItemDecision::skipped(LibraryFileReprocessReason::NotFound),
                );
            }
        }
        let mut admitted = Vec::new();
        for file_id in &owned {
            let Some(lifecycle) = self
                .library()
                .lock_file_source_facts(&mut transaction, *file_id)
                .await?
            else {
                // The file vanished between the fact read and the lock.
                decisions.insert(
                    *file_id,
                    ItemDecision::skipped(LibraryFileReprocessReason::NotFound),
                );
                continue;
            };
            let source_uri = lifecycle.source_uri.as_deref();
            if !can_reprocess_source(
                &lifecycle.ingest_status,
                &lifecycle.filename,
                lifecycle.external_id.as_deref(),
                lifecycle.source_released_at.is_some(),
                source_uri,
            ) {
                let reason = if lifecycle.ingest_status != "succeeded" {
                    LibraryFileReprocessReason::NotSucceeded
                } else if crate::services::library::is_managed_source_file(
                    &lifecycle.filename,
                    lifecycle.external_id.as_deref(),
                ) {
                    LibraryFileReprocessReason::SourceProtected
                } else {
                    LibraryFileReprocessReason::SourceUnavailable
                };
                decisions.insert(*file_id, ItemDecision::skipped(reason));
                continue;
            }
            // The durable facts alone are not proof that the bytes are there:
            // an admission must not claim a retained source it cannot read.
            // A storage dependency failure is an outage, not absence, so the
            // file is reported as failed instead of being called unavailable.
            let retained = match self.library().retained_bytes_present(&lifecycle).await {
                Ok(present) => present,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        file_id = %file_id,
                        "file reprocess could not verify the retained source object"
                    );
                    decisions.insert(
                        *file_id,
                        ItemDecision::failed(LibraryFileReprocessReason::AdmissionFailed),
                    );
                    continue;
                }
            };
            if !retained && !has_downloadable_source(source_uri) {
                decisions.insert(
                    *file_id,
                    ItemDecision::skipped(LibraryFileReprocessReason::SourceUnavailable),
                );
                continue;
            }
            admitted.push(*file_id);
        }

        let active = active_file_task_items(&mut transaction, request.group_id, &admitted).await?;
        let mut accepted = Vec::new();
        for file_id in admitted {
            if active.contains_key(&file_id) {
                decisions.insert(
                    file_id,
                    ItemDecision::skipped(LibraryFileReprocessReason::ActiveSourceBusy),
                );
                continue;
            }
            accepted.push(file_id);
        }

        let created = if accepted.is_empty() {
            None
        } else {
            let items = accepted
                .iter()
                .map(|file_id| NewFileReprocessItem {
                    file_id: *file_id,
                    payload: crate::chunking::payload_with_chunk_budget(
                        &serde_json::json!({
                            "file_id": file_id.to_string(),
                            SOURCE_FALLBACK_PAYLOAD_KEY: SOURCE_FALLBACK_URL,
                        }),
                        &budget,
                    ),
                })
                .collect::<Vec<_>>();
            Some(
                self.db()
                    .create_file_reprocess_task_in(
                        &mut transaction,
                        request.user_id,
                        request.group_id,
                        request.group_path,
                        &items,
                    )
                    .await?,
            )
        };
        transaction.commit().await?;

        if created.is_some() {
            self.notify_dispatch();
        }
        let (task_id, item_count) = match created {
            Some(created) => (Some(created.task_id), created.item_ids.len()),
            None => (None, 0),
        };
        if let Some(task_id) = task_id {
            tracing::info!(
                task_id = %task_id,
                group_id = request.group_id,
                file_count = accepted.len(),
                item_count,
                "context69 file reprocess task created"
            );
        }

        let results = request
            .file_ids
            .iter()
            .map(|file_id| match decisions.get(file_id) {
                Some(decision) => LibraryFileReprocessResult {
                    file_id: *file_id,
                    outcome: decision.outcome,
                    reason: Some(decision.reason),
                    task_id: None,
                },
                None => LibraryFileReprocessResult {
                    file_id: *file_id,
                    outcome: LibraryFileReprocessOutcome::Accepted,
                    reason: None,
                    task_id,
                },
            })
            .collect::<Vec<_>>();
        Ok(ReprocessLibraryFilesResponse {
            results,
            task_ids: task_id.into_iter().collect(),
        })
    }
}

/// Reject a malformed batch before any effect.
fn validate_request(file_ids: &[Uuid]) -> Result<()> {
    if file_ids.is_empty() {
        return Err(
            DomainError::invalid_argument("file reprocess requires at least one file id").into(),
        );
    }
    if file_ids.len() > MAX_REPROCESS_FILE_IDS {
        return Err(DomainError::invalid_argument(format!(
            "file reprocess accepts at most {MAX_REPROCESS_FILE_IDS} file ids"
        ))
        .into());
    }
    let mut unique = file_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != file_ids.len() {
        return Err(DomainError::invalid_argument("file reprocess file ids must be unique").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_batch_is_rejected() {
        assert!(validate_request(&[]).is_err());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let file_id = Uuid::new_v4();
        assert!(validate_request(&[file_id, file_id]).is_err());
    }

    #[test]
    fn an_oversized_batch_is_rejected() {
        let file_ids = (0..=MAX_REPROCESS_FILE_IDS)
            .map(|_| Uuid::new_v4())
            .collect::<Vec<_>>();
        assert!(validate_request(&file_ids).is_err());
    }

    #[test]
    fn a_bounded_unique_batch_is_accepted() {
        let file_ids = (0..MAX_REPROCESS_FILE_IDS)
            .map(|_| Uuid::new_v4())
            .collect::<Vec<_>>();
        assert!(validate_request(&file_ids).is_ok());
    }

    #[test]
    fn only_http_sources_are_downloadable() {
        assert!(crate::services::library::has_downloadable_source(Some(
            "https://example.test/a.pdf"
        )));
        assert!(crate::services::library::has_downloadable_source(Some(
            "HTTP://example.test/a.pdf"
        )));
        assert!(!crate::services::library::has_downloadable_source(Some(
            "ftp://example.test/a.pdf"
        )));
        assert!(!crate::services::library::has_downloadable_source(Some(
            "ééééééééé"
        )));
        assert!(!crate::services::library::has_downloadable_source(None));
    }

    #[test]
    fn eligibility_needs_a_succeeded_unprotected_source() {
        assert!(can_reprocess_source(
            "succeeded",
            "a.pdf",
            None,
            false,
            None
        ));
        assert!(!can_reprocess_source("failed", "a.pdf", None, false, None));
        assert!(!can_reprocess_source(
            "succeeded",
            "source.json",
            None,
            false,
            None
        ));
        assert!(!can_reprocess_source(
            "succeeded",
            "a.pdf",
            Some("source-folder:1"),
            false,
            None
        ));
        assert!(!can_reprocess_source(
            "succeeded",
            "a.pdf",
            None,
            true,
            None
        ));
        assert!(can_reprocess_source(
            "succeeded",
            "a.pdf",
            None,
            true,
            Some("https://example.test/a.pdf")
        ));
    }
}
