//! Deterministic file eligibility for Git snapshots (issue #28 phase P3).
//!
//! Selection is pure and path-based: it decides from the tree entry alone which
//! files may be fetched, so a listing can be reduced before any blob byte is
//! spent. Safety exclusions (version-control and dependency directories, build
//! outputs, binary, minified, source-map, lock, and secret files) cannot be
//! re-enabled by a user pattern; the per-file byte ceiling and the snapshot
//! counts stay hard limits applied afterwards by [`super::limits`].
//!
//! Default eligibility is the bounded source/docs/manifest allowlist the
//! product indexes. A user include pattern may select another textual file, and
//! a user exclude always wins over both the default allowlist and a user
//! include. Skips are counted per reason so partial coverage is visible.

use std::collections::BTreeMap;

use crate::db::MAX_GIT_FILE_BYTES;

use super::model::TreeFileEntry;

/// Repository directory names that never contribute indexed content.
const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".gradle",
];

/// File extensions that carry binary or generated payloads, never source text.
const BINARY_EXTENSIONS: &[&str] = &[
    "7z", "a", "avi", "bin", "bmp", "class", "dll", "dmg", "doc", "docx", "eot", "exe", "flac",
    "gif", "gz", "ico", "jar", "jpeg", "jpg", "m4a", "mkv", "mov", "mp3", "mp4", "o", "obj", "ogg",
    "otf", "pdf", "png", "pyc", "so", "sqlite", "sqlite3", "tar", "tif", "tiff", "ttf", "wav",
    "webm", "webp", "woff", "woff2", "xls", "xlsx", "xz", "zip", "zst",
];

/// Whole names and suffixes that are lockfiles rather than source text.
const LOCKFILE_NAMES: &[&str] = &[
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "composer.lock",
    "deno.lock",
    "flake.lock",
    "gemfile.lock",
    "go.sum",
    "mix.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "podfile.lock",
    "poetry.lock",
    "yarn.lock",
];

/// Names that carry credentials or credential material.
const SECRET_NAMES: &[&str] = &[
    ".env",
    ".netrc",
    ".npmrc",
    ".pgpass",
    ".pypirc",
    "credentials",
    "credentials.json",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_rsa",
];

/// Extensions that carry private keys, certificates, or keystores.
const SECRET_EXTENSIONS: &[&str] = &["csr", "der", "jks", "key", "keystore", "p12", "pem", "pfx"];

/// Source extensions the default allowlist admits.
const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "sql"];

/// Documentation extensions the default allowlist admits.
const DOCUMENT_EXTENSIONS: &[&str] = &["md", "mdx", "rst", "txt"];

/// Manifest file names the default allowlist admits.
const MANIFEST_NAMES: &[&str] = &["cargo.toml", "package.json"];

/// Why one tree entry is not selected for indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SkipReason {
    /// A version-control, dependency, or build-output directory.
    ExcludedDirectory,
    /// A binary or generated payload extension.
    Binary,
    /// A minified bundle.
    Minified,
    /// A source map.
    SourceMap,
    /// A lockfile.
    Lockfile,
    /// A credential or credential material.
    Secret,
    /// A declared size above the per-file storage ceiling.
    TooLarge,
    /// A user exclude pattern matched.
    ExcludedByUser,
    /// Neither default eligibility nor a user include admitted the path.
    NotIncluded,
}

impl SkipReason {
    /// Stable lowercase token recorded in skip summaries.
    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExcludedDirectory => "excluded_directory",
            Self::Binary => "binary",
            Self::Minified => "minified",
            Self::SourceMap => "source_map",
            Self::Lockfile => "lockfile",
            Self::Secret => "secret",
            Self::TooLarge => "too_large",
            Self::ExcludedByUser => "excluded_by_user",
            Self::NotIncluded => "not_included",
        }
    }
}

/// User-configurable glob patterns layered over default eligibility.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SelectionRules {
    /// Additional path patterns admitted beyond the default allowlist.
    pub includes: Vec<String>,
    /// Patterns excluded even when the default allowlist or an include matches.
    pub excludes: Vec<String>,
}

impl SelectionRules {
    pub(crate) fn new(includes: Vec<String>, excludes: Vec<String>) -> Self {
        Self { includes, excludes }
    }
}

/// Reason for one selection decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionDecision {
    Include,
    Skip(SkipReason),
}

/// Counts of skipped entries per reason, in stable reason order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SkipSummary {
    counts: BTreeMap<SkipReason, usize>,
}

impl SkipSummary {
    pub(crate) fn record(&mut self, reason: SkipReason) {
        *self.counts.entry(reason).or_insert(0) += 1;
    }

    pub(crate) fn count(&self, reason: SkipReason) -> usize {
        self.counts.get(&reason).copied().unwrap_or(0)
    }

    pub(crate) fn total(&self) -> usize {
        self.counts.values().sum()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// Selected entries plus the per-reason skip counts of everything rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectionOutcome {
    pub files: Vec<TreeFileEntry>,
    pub skipped: SkipSummary,
}

/// Decides one tree entry from its path and declared size.
///
/// Safety exclusions run first and cannot be overridden. A user exclude then
/// wins over the default allowlist and a user include. With no user include
/// patterns, default eligibility decides; otherwise default eligibility or a
/// matching include admits the path.
pub(crate) fn selection_decision(
    path: &str,
    declared_size: Option<u64>,
    rules: &SelectionRules,
) -> SelectionDecision {
    if let Some(reason) = safety_skip_reason(path) {
        return SelectionDecision::Skip(reason);
    }
    if declared_size.is_some_and(|size| size > MAX_GIT_FILE_BYTES as u64) {
        return SelectionDecision::Skip(SkipReason::TooLarge);
    }
    if rules
        .excludes
        .iter()
        .any(|pattern| glob_matches(pattern, path))
    {
        return SelectionDecision::Skip(SkipReason::ExcludedByUser);
    }
    if default_eligible(path)
        || rules
            .includes
            .iter()
            .any(|pattern| glob_matches(pattern, path))
    {
        return SelectionDecision::Include;
    }
    SelectionDecision::Skip(SkipReason::NotIncluded)
}

/// Applies the selection decision to one listing.
pub(crate) fn select_snapshot_files(
    entries: Vec<TreeFileEntry>,
    rules: &SelectionRules,
) -> SelectionOutcome {
    let mut files = Vec::with_capacity(entries.len());
    let mut skipped = SkipSummary::default();
    for entry in entries {
        match selection_decision(entry.path.as_str(), entry.size, rules) {
            SelectionDecision::Include => files.push(entry),
            SelectionDecision::Skip(reason) => skipped.record(reason),
        }
    }
    SelectionOutcome { files, skipped }
}

fn safety_skip_reason(path: &str) -> Option<SkipReason> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower_name = name.to_ascii_lowercase();
    if path
        .split('/')
        .any(|segment| EXCLUDED_DIRECTORIES.contains(&segment))
    {
        return Some(SkipReason::ExcludedDirectory);
    }
    if SECRET_NAMES.contains(&lower_name.as_str()) || lower_name.starts_with(".env.") {
        return Some(SkipReason::Secret);
    }
    if LOCKFILE_NAMES.contains(&lower_name.as_str()) {
        return Some(SkipReason::Lockfile);
    }
    if lower_name.contains(".min.") {
        return Some(SkipReason::Minified);
    }
    let Some((_, extension)) = lower_name.rsplit_once('.') else {
        return None;
    };
    if extension == "map" {
        return Some(SkipReason::SourceMap);
    }
    if extension == "lock" {
        return Some(SkipReason::Lockfile);
    }
    if SECRET_EXTENSIONS.contains(&extension) {
        return Some(SkipReason::Secret);
    }
    if BINARY_EXTENSIONS.contains(&extension) {
        return Some(SkipReason::Binary);
    }
    None
}

fn default_eligible(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower_name = name.to_ascii_lowercase();
    if MANIFEST_NAMES.contains(&lower_name.as_str()) {
        return true;
    }
    let Some((_, extension)) = lower_name.rsplit_once('.') else {
        return false;
    };
    SOURCE_EXTENSIONS.contains(&extension) || DOCUMENT_EXTENSIONS.contains(&extension)
}

/// Matches one repository-relative glob pattern against a path.
///
/// `**` spans directory separators, `*` and `?` stay inside one path segment,
/// and a pattern without a slash may match any single segment, so `*.md`
/// selects every Markdown file. A leading `/` anchors nothing extra because
/// paths are already repository-relative.
pub(crate) fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_start_matches('/');
    if pattern.is_empty() || pattern.len() > 512 {
        return false;
    }
    if pattern.contains('/') {
        let pattern_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if pattern_segments.len() > 64 || path_segments.len() > 64 {
            return false;
        }
        return match_segments(&pattern_segments, &path_segments);
    }
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .any(|segment| match_segment(pattern, segment))
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    let mut memo = vec![vec![None; path.len() + 1]; pattern.len() + 1];
    match_segments_at(pattern, path, 0, 0, &mut memo)
}

fn match_segments_at(
    pattern: &[&str],
    path: &[&str],
    pattern_index: usize,
    path_index: usize,
    memo: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(result) = memo[pattern_index][path_index] {
        return result;
    }
    let result = match pattern.get(pattern_index) {
        None => path_index == path.len(),
        Some(&"**") => (path_index..=path.len())
            .any(|skip| match_segments_at(pattern, path, pattern_index + 1, skip, memo)),
        Some(segment) => path.get(path_index).is_some_and(|candidate| {
            match_segment(segment, candidate)
                && match_segments_at(pattern, path, pattern_index + 1, path_index + 1, memo)
        }),
    };
    memo[pattern_index][path_index] = Some(result);
    result
}

fn match_segment(pattern: &str, segment: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let segment: Vec<char> = segment.chars().collect();
    let (mut pattern_index, mut segment_index) = (0, 0);
    let (mut star_pattern, mut star_segment) = (None, 0);
    while segment_index < segment.len() {
        match pattern.get(pattern_index) {
            Some('*') => {
                star_pattern = Some(pattern_index);
                star_segment = segment_index;
                pattern_index += 1;
            }
            Some('?') => {
                pattern_index += 1;
                segment_index += 1;
            }
            Some(&character) if character == segment[segment_index] => {
                pattern_index += 1;
                segment_index += 1;
            }
            _ => match star_pattern {
                Some(star) => {
                    pattern_index = star + 1;
                    star_segment += 1;
                    segment_index = star_segment;
                }
                None => return false,
            },
        }
    }
    pattern[pattern_index..]
        .iter()
        .all(|&character| character == '*')
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
