//! Deterministic file-level snapshot planning (issue #28 phase P2).
//!
//! A new target is compared with the active generation's manifest by
//! repository path and provider blob id, never by fetching content. A path
//! with the same blob id is unchanged and can be seeded from the baseline; a
//! changed or added path must be fetched and processed; a path the new listing
//! no longer selects simply disappears from the new generation. The comparison
//! is bounded by the already-selected listing and the stored manifest size.

use std::collections::BTreeMap;

use super::model::TreeFileEntry;

/// One path whose baseline rows and bytes are copied unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeededFile {
    pub path: String,
    pub provider_blob_sha: String,
    /// Baseline row's stored byte count, so coverage stays exact without
    /// reading the bytes again.
    pub byte_count: i64,
}

/// What one target listing means relative to the baseline manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SnapshotPlan {
    pub seeded: Vec<SeededFile>,
    pub fetched: Vec<TreeFileEntry>,
    pub deleted_path_count: usize,
}

impl SnapshotPlan {
    pub(crate) fn reused_file_count(&self) -> usize {
        self.seeded.len()
    }

    pub(crate) fn changed_file_count(&self) -> usize {
        self.fetched.len()
    }
}

/// The baseline manifest projected to exactly path and blob id plus byte count.
pub(crate) type BaselineManifest = BTreeMap<String, (String, i64)>;

/// Compares the selected target entries with the baseline manifest.
///
/// Paths are unique in both inputs, so one pass over the target decides every
/// addition, change, and copy, and the leftover baseline paths are the
/// deletions. Nothing here decides eligibility: the target list has already
/// passed selection and limits, and the baseline is the manifest that actually
/// served the previous snapshot.
pub(crate) fn plan_incremental_snapshot(
    target: Vec<TreeFileEntry>,
    baseline: &BaselineManifest,
) -> SnapshotPlan {
    let mut plan = SnapshotPlan::default();
    let mut remaining: std::collections::BTreeSet<&str> =
        baseline.keys().map(String::as_str).collect();
    for entry in target {
        let path = entry.path.as_str();
        match baseline.get(path) {
            Some((blob_sha, byte_count)) if blob_sha == entry.blob_sha.as_str() => {
                remaining.remove(path);
                plan.seeded.push(SeededFile {
                    path: path.to_string(),
                    provider_blob_sha: blob_sha.clone(),
                    byte_count: *byte_count,
                });
            }
            Some(_) => {
                remaining.remove(path);
                plan.fetched.push(entry);
            }
            None => plan.fetched.push(entry),
        }
    }
    plan.deleted_path_count = remaining.len();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::git_repository::ref_path_safety::{SafeSha, SafeTreePath};

    fn entry(path: &str, blob: &str) -> TreeFileEntry {
        TreeFileEntry {
            path: SafeTreePath::parse(path).expect("path"),
            blob_sha: SafeSha::parse(blob).expect("sha"),
            size: None,
        }
    }

    fn sha(seed: u8) -> String {
        format!("{seed:02x}").repeat(20)
    }

    #[test]
    fn unchanged_modified_added_and_deleted_are_classified_once() {
        let baseline = BaselineManifest::from([
            ("src/keep.rs".to_string(), (sha(1), 10)),
            ("src/change.rs".to_string(), (sha(2), 20)),
            ("src/delete.rs".to_string(), (sha(3), 30)),
        ]);
        let plan = plan_incremental_snapshot(
            vec![
                entry("src/keep.rs", &sha(1)),
                entry("src/change.rs", &sha(4)),
                entry("src/add.rs", &sha(5)),
            ],
            &baseline,
        );

        assert_eq!(
            plan.seeded,
            vec![SeededFile {
                path: "src/keep.rs".to_string(),
                provider_blob_sha: sha(1),
                byte_count: 10,
            }]
        );
        assert_eq!(
            plan.fetched
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src/change.rs", "src/add.rs"]
        );
        assert_eq!(plan.reused_file_count(), 1);
        assert_eq!(plan.changed_file_count(), 2);
        assert_eq!(plan.deleted_path_count, 1);
    }

    #[test]
    fn an_empty_target_deletes_every_baseline_path() {
        let baseline = BaselineManifest::from([("a.rs".to_string(), (sha(1), 10))]);
        let plan = plan_incremental_snapshot(Vec::new(), &baseline);
        assert!(plan.seeded.is_empty() && plan.fetched.is_empty());
        assert_eq!(plan.deleted_path_count, 1);
    }
}
