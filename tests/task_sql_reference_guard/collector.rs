//! Filesystem inventory and exact macro-path resolution for the task SQL guard.
//!
//! The collector fails hard on any filesystem error so a broken subtree can
//! never shrink the inventory, and a macro path is resolved against its real
//! base (crate root or containing source file) before it is compared with the
//! enumerated task SQL file.

use std::path::{Path, PathBuf};

use super::parser::{SqlMacro, macro_path_literals};

/// Push every file under `dir` whose extension is `extension`, recursing into
/// subdirectories.
///
/// Directory metadata is inspected with an explicit fallible `symlink_metadata`
/// call, and any error — a missing, disappearing, or unreadable directory or
/// entry — panics with context instead of being reduced to a boolean by
/// `Path::is_dir()`, so a broken subtree can never shrink the inventory.
pub(crate) fn collect_by_extension(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read directory {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("read directory entry under {}: {error}", dir.display())
        });
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).unwrap_or_else(|error| {
            panic!(
                "stat {} collected under {}: {error}",
                path.display(),
                dir.display()
            )
        });
        if metadata.is_dir() {
            collect_by_extension(&path, extension, out);
        } else if path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
}

/// Lexically resolves `.` and `..` components of `path` without touching the
/// filesystem, which is exactly how the compiler resolves a macro path relative
/// to its base directory. `path` is expected to be absolute.
pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// The directory a macro path is resolved against: the crate root for
/// `sqlx::query_file*!`, the containing source file's directory for
/// `include_str!`.
fn macro_base<'a>(root: &'a Path, source_path: &'a Path, kind: SqlMacro) -> &'a Path {
    match kind {
        SqlMacro::QueryFileFirst | SqlMacro::QueryFileAsSecond => root,
        SqlMacro::IncludeStr => source_path.parent().unwrap_or(root),
    }
}

/// Whether `source` reaches the enumerated task SQL file `target` through the
/// path argument of a recognized macro.
///
/// The literal is resolved against its real base (crate root or source file
/// directory), `..`/`.` are resolved, and the result must equal `target`
/// exactly. A literal that merely shares a suffix but resolves elsewhere is
/// rejected.
pub(crate) fn references_task_sql(
    root: &Path,
    source_path: &Path,
    source: &str,
    target: &Path,
) -> bool {
    macro_path_literals(source).iter().any(|(kind, literal)| {
        let base = macro_base(root, source_path, *kind);
        normalize_lexical(&base.join(literal)) == target
    })
}
