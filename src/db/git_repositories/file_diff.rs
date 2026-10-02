//! Metadata-only comparison of two stored generation manifests (issue #681 phase
//! 5E).
//!
//! One bounded page of changed paths, in path order, between two index
//! generations of one repository. The comparison is decided in SQL: the two
//! manifests are joined on the repository-relative path and the stored provider
//! content addresses are compared there, so a provider blob id is never selected
//! into a row, a response, or a test. Everything this module projects is safe
//! manifest metadata — the path, the classified language, and the byte and line
//! counts of each side — so a caller can size a change without reading a file,
//! a chunk, or a raw blob.
//!
//! The read is confined twice over, and the statement enforces the pair itself:
//! both generation keys must name a completed generation (`ready` or
//! `superseded`) of the calling group and repository, or the read answers no rows
//! at all. That is what keeps a caller-chosen key from turning into a diff — a
//! full outer join against one empty side would otherwise report every path of the
//! other side as `added` or `deleted`. Each side keeps its own group and
//! repository filter, and a pointer that changed between resolution and this read
//! cannot widen the comparison either. An unchanged path is omitted rather than
//! reported, so an empty page is the truthful answer both for two generations that
//! hold the same bytes at the same paths and for a pair that is not comparable.

use anyhow::Result;
use sqlx::FromRow;
use uuid::Uuid;

use super::files::bounded_page;
use crate::contracts::sources::GitFileChangeKind;
use crate::db::Database;
use crate::domain_errors::DomainError;

/// One row of the comparison, as the statement projects it.
///
/// The row carries no provider blob id: the content addresses were compared in
/// SQL and only their equality survived, so there is nothing here a caller could
/// use to address a raw blob.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitFileDiffRow {
    pub path: String,
    pub change_kind: String,
    pub before_file_key: Option<Uuid>,
    pub before_language: Option<String>,
    pub before_byte_count: Option<i64>,
    pub before_line_count: Option<i64>,
    pub after_file_key: Option<Uuid>,
    pub after_language: Option<String>,
    pub after_byte_count: Option<i64>,
    pub after_line_count: Option<i64>,
}

/// The safe manifest metadata of one side of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileDiffSide {
    pub file_key: Uuid,
    pub language: String,
    pub byte_count: i64,
    pub line_count: i64,
}

/// One changed repository-relative path between two generations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitFileDiff {
    pub path: String,
    pub change_kind: GitFileChangeKind,
    /// Entry of the compared-from generation; absent when the path was added.
    pub before: Option<GitFileDiffSide>,
    /// Entry of the compared-to generation; absent when the path was deleted.
    pub after: Option<GitFileDiffSide>,
}

impl StoredGitFileDiff {
    /// Build the stored comparison from one row, failing closed.
    ///
    /// A change kind this storage layer never writes, a side that is present
    /// without its key, and an `Added` or `Deleted` path that arrives with both
    /// sides are all refused instead of being projected as something a caller
    /// would read as a real change: a diff that cannot be trusted is better than
    /// one that reports a side that does not exist.
    pub(crate) fn from_row(row: GitFileDiffRow) -> Result<Self> {
        let change_kind = GitFileChangeKind::from_wire(&row.change_kind)
            .ok_or_else(|| DomainError::internal("git_diff_change_kind_invalid"))?;
        let before = side(
            row.before_file_key,
            row.before_language,
            row.before_byte_count,
            row.before_line_count,
        )?;
        let after = side(
            row.after_file_key,
            row.after_language,
            row.after_byte_count,
            row.after_line_count,
        )?;
        if (change_kind == GitFileChangeKind::Added && before.is_some())
            || (change_kind == GitFileChangeKind::Deleted && after.is_some())
        {
            return Err(DomainError::internal("git_diff_sides_unexpected").into());
        }
        Ok(Self {
            path: row.path,
            change_kind,
            before,
            after,
        })
    }
}

/// One side of a change, or an error when a stored row is internally
/// inconsistent: a present key with a missing count cannot be projected as
/// metadata a caller could trust.
fn side(
    file_key: Option<Uuid>,
    language: Option<String>,
    byte_count: Option<i64>,
    line_count: Option<i64>,
) -> Result<Option<GitFileDiffSide>> {
    match (file_key, language, byte_count, line_count) {
        (None, None, None, None) => Ok(None),
        (Some(file_key), Some(language), Some(byte_count), Some(line_count)) => {
            Ok(Some(GitFileDiffSide {
                file_key,
                language,
                byte_count,
                line_count,
            }))
        }
        _ => Err(DomainError::internal("git_diff_side_incomplete").into()),
    }
}

impl Database {
    /// Reads one bounded page of changed paths between two generations.
    ///
    /// Both keys are the caller's claims, so the statement re-checks them: the read
    /// answers rows only when *both* name a generation of this repository owned by
    /// this group whose status is comparable — the active `ready` one or one a
    /// newer snapshot `superseded`. A key that is missing, foreign, of another
    /// repository, still building, or failed therefore yields no rows rather than a
    /// one-sided diff, which is the difference between "nothing to compare" and a
    /// fabricated list of additions. The same bounds as the manifest listing apply,
    /// so this read can never become an unbounded listing of a repository's
    /// history.
    pub async fn list_git_generation_file_diff(
        &self,
        group_id: i64,
        repository_key: Uuid,
        from_generation_key: Uuid,
        to_generation_key: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredGitFileDiff>> {
        bounded_page(limit, offset)?;
        let rows = sqlx::query_file_as!(
            GitFileDiffRow,
            "src/sql/db/git_repository_files/list_git_generation_file_diff.sql",
            group_id,
            repository_key,
            from_generation_key,
            to_generation_key,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(StoredGitFileDiff::from_row).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{GitFileDiffRow, GitFileDiffSide, StoredGitFileDiff};
    use crate::contracts::sources::GitFileChangeKind;
    use uuid::Uuid;

    fn file_key(last: &str) -> Uuid {
        Uuid::parse_str(last).expect("uuid")
    }

    fn row(change_kind: &str, with_before: bool, with_after: bool) -> GitFileDiffRow {
        GitFileDiffRow {
            path: "src/api/mod.rs".to_string(),
            change_kind: change_kind.to_string(),
            before_file_key: with_before.then(|| file_key("018f9f40-1111-7000-8000-0000000000c1")),
            before_language: with_before.then(|| "rust".to_string()),
            before_byte_count: with_before.then_some(512),
            before_line_count: with_before.then_some(20),
            after_file_key: with_after.then(|| file_key("018f9f40-2222-7000-8000-0000000000c2")),
            after_language: with_after.then(|| "rust".to_string()),
            after_byte_count: with_after.then_some(640),
            after_line_count: with_after.then_some(24),
        }
    }

    #[test]
    fn every_change_kind_keeps_only_the_side_it_owns() {
        let added = StoredGitFileDiff::from_row(row("added", false, true)).expect("an added path");
        assert_eq!(added.change_kind, GitFileChangeKind::Added);
        assert!(added.before.is_none());
        assert_eq!(
            added.after,
            Some(GitFileDiffSide {
                file_key: file_key("018f9f40-2222-7000-8000-0000000000c2"),
                language: "rust".to_string(),
                byte_count: 640,
                line_count: 24
            })
        );

        let deleted = StoredGitFileDiff::from_row(row("deleted", true, false)).expect("a deletion");
        assert_eq!(deleted.change_kind, GitFileChangeKind::Deleted);
        assert!(deleted.after.is_none());
        assert_eq!(deleted.before.map(|side| side.byte_count), Some(512));

        let modified = StoredGitFileDiff::from_row(row("modified", true, true)).expect("a change");
        assert_eq!(modified.change_kind, GitFileChangeKind::Modified);
        assert_eq!(modified.path, "src/api/mod.rs");
        assert!(modified.before.is_some() && modified.after.is_some());
    }

    #[test]
    fn a_row_this_layer_cannot_trust_is_refused() {
        for change_kind in ["unchanged", "", "MODIFIED", "renamed"] {
            let error = StoredGitFileDiff::from_row(row(change_kind, true, true))
                .expect_err("a kind this layer never writes is refused")
                .to_string();
            assert!(
                error.contains("git_diff_change_kind_invalid"),
                "kind {change_kind:?} must be refused: {error}"
            );
        }
        // An added path that also carries the before side, and a half-written
        // side, are both refused rather than projected.
        assert!(
            StoredGitFileDiff::from_row(row("added", true, true))
                .expect_err("an added path has no before side")
                .to_string()
                .contains("git_diff_sides_unexpected")
        );
        assert!(
            StoredGitFileDiff::from_row(row("deleted", true, true))
                .expect_err("a deleted path has no after side")
                .to_string()
                .contains("git_diff_sides_unexpected")
        );
        let mut partial = row("modified", true, true);
        partial.after_line_count = None;
        assert!(
            StoredGitFileDiff::from_row(partial)
                .expect_err("a side without its line count is refused")
                .to_string()
                .contains("git_diff_side_incomplete")
        );
    }
}
