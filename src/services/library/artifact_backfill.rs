//! Explicit preview and bounded terminal artifact backfill (issue #975 P5).
//!
//! The default invocation previews terminal inline artifacts with a
//! read-only PostgreSQL session and metadata-only queries. Nothing here runs on
//! application startup: a caller must construct the arguments and call
//! [`run_preview`] or [`run_apply`] from the operator binary.
//!
//! Apply is fail-closed: it refuses to start unless the compiled #975 migration
//! is applied with its exact checksum, the artifact relation exists, and the
//! caller passed both `--apply` and `--acknowledge-reader-upgrade`. One artifact
//! is staged, read back and verified at a time; the terminal CAS strips only the
//! migrated inline field after the verified reference is durable.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use context69_translation::{TranslationDependencies, TranslationService};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::{Config, DATABASE_URL_ENV_VAR};
use crate::db::{
    ArtifactBackfillCursor, ArtifactBackfillRow, ArtifactRestoreRow,
    AttachTerminalTaskItemArtifactRequest, Database, MIGRATOR, RemainingArtifactReferences,
    RestoreTaskItemArtifactInlineRequest, StoredRuntimeSettings, TaskItemArtifactKind,
};
use crate::embedding::EmbeddingRuntime;
use crate::services::app;
use crate::services::library::{
    LibraryService, LibraryServiceConfig, MAX_TASK_ITEM_ARTIFACT_BYTES,
};
use crate::services::secret_store;
use crate::services::settings::SettingsService;

use super::IngestSection;

/// The migration whose applied checksum gates apply.
pub const ARTIFACT_MIGRATION_VERSION: i64 = 20261019104722;

/// Stable acknowledgement text for the forward apply gate.
pub const APPLY_ACKNOWLEDGEMENT: &str = "This tool removes inline artifact bytes from terminal task items. \
Rolling the application back to a build without artifact-aware readers is unsafe while \
externalized items exist; run the restore-inline mode first and keep the artifact-aware \
version deployed.";

/// Stable acknowledgement text for the inverse apply gate.
pub const RESTORE_ACKNOWLEDGEMENT: &str = "This tool copies verified artifact bytes back into terminal task \
items and can grow PostgreSQL storage. It does not roll back the schema or migration ledger, \
it does not make an older application version safe to deploy, and it only restores items whose \
inline field is currently missing.";

/// Which data direction a run applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactDirection {
    /// Inline bytes to the content-addressed artifact store.
    Forward,
    /// Verified artifact bytes back into the missing inline field.
    RestoreInline,
}

impl ArtifactDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::RestoreInline => "restore_inline",
        }
    }

    fn acknowledgement(self) -> &'static str {
        match self {
            Self::Forward => APPLY_ACKNOWLEDGEMENT,
            Self::RestoreInline => RESTORE_ACKNOWLEDGEMENT,
        }
    }
}

const DEFAULT_PAGE_SIZE: i64 = 100;
const MAX_PAGE_SIZE: i64 = 1000;
const DEFAULT_MAX_ITEMS: i64 = 500;
const MAX_MAX_ITEMS: i64 = 1_000_000;
const DEFAULT_MAX_ARTIFACT_BYTES: i64 = MAX_TASK_ITEM_ARTIFACT_BYTES as i64;
const MAX_ARTIFACT_BYTES_HARD_CAP: i64 = MAX_TASK_ITEM_ARTIFACT_BYTES as i64;
const DEFAULT_MAX_TOTAL_BYTES: i64 = 512 * 1024 * 1024;
const MAX_TOTAL_BYTES_HARD_CAP: i64 = 1024 * 1024 * 1024 * 1024;
const DEFAULT_DELAY_MS: u64 = 100;
const MAX_DELAY_MS: u64 = 60_000;
const MAX_ITEM_IDS: usize = 1000;
const SAMPLE_LIMIT: usize = 10;
const BASE64_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Which artifact kinds a run may process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKindFilter {
    Download,
    Sections,
    All,
}

impl ArtifactKindFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Sections => "sections",
            Self::All => "all",
        }
    }

    fn includes(self, kind: TaskItemArtifactKind) -> bool {
        matches!(
            (self, kind),
            (Self::All, _)
                | (Self::Download, TaskItemArtifactKind::Download)
                | (Self::Sections, TaskItemArtifactKind::Sections)
        )
    }
}

/// Parsed operator flags. Private field shapes stay internal; the documented
/// flag spellings are the stable interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBackfillArgs {
    pub apply: bool,
    pub restore_inline: bool,
    pub acknowledge_reader_upgrade: bool,
    pub acknowledge_inline_growth: bool,
    pub kind: ArtifactKindFilter,
    pub item_ids: Vec<Uuid>,
    pub page_size: i64,
    pub max_items: i64,
    pub max_artifact_bytes: i64,
    pub max_total_bytes: i64,
    pub delay_ms: u64,
    pub resume_cursor: Option<ArtifactBackfillCursor>,
    pub help: bool,
}

impl ArtifactBackfillArgs {
    /// The direction these flags select.
    pub fn direction(&self) -> ArtifactDirection {
        if self.restore_inline {
            ArtifactDirection::RestoreInline
        } else {
            ArtifactDirection::Forward
        }
    }
}

impl Default for ArtifactBackfillArgs {
    fn default() -> Self {
        Self {
            apply: false,
            restore_inline: false,
            acknowledge_reader_upgrade: false,
            acknowledge_inline_growth: false,
            kind: ArtifactKindFilter::All,
            item_ids: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            max_items: DEFAULT_MAX_ITEMS,
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            delay_ms: DEFAULT_DELAY_MS,
            resume_cursor: None,
            help: false,
        }
    }
}

/// Parse operator arguments without opening a connection.
pub fn parse_args<I, S>(raw: I) -> Result<ArtifactBackfillArgs>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = ArtifactBackfillArgs::default();
    let mut raw = raw.into_iter().map(Into::into).peekable();
    let mut seen_item_ids = BTreeSet::new();
    while let Some(argument) = raw.next() {
        let (flag, inline_value) = match argument.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (argument, None),
        };
        if is_presence_only_flag(&flag) {
            if let Some(inline) = inline_value.as_deref() {
                bail!(
                    "{flag} is presence-only and does not take a value (got {inline:?}); pass only \
                     {flag}"
                );
            }
            if let Some(next) = raw.peek()
                && is_boolean_word(next)
            {
                bail!(
                    "{flag} is presence-only and does not take a value; remove the trailing {next:?}"
                );
            }
        }
        let mut value = || -> Result<String> {
            if let Some(value) = inline_value.clone() {
                return Ok(value);
            }
            raw.next()
                .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))
        };
        match flag.as_str() {
            "--apply" => args.apply = true,
            "--restore-inline" => args.restore_inline = true,
            "--acknowledge-reader-upgrade" => args.acknowledge_reader_upgrade = true,
            "--acknowledge-inline-growth" => args.acknowledge_inline_growth = true,
            "--kind" => {
                args.kind = match value()?.as_str() {
                    "download" => ArtifactKindFilter::Download,
                    "sections" => ArtifactKindFilter::Sections,
                    "all" => ArtifactKindFilter::All,
                    other => bail!("--kind must be download, sections or all, got {other}"),
                }
            }
            "--item-id" => {
                let parsed = Uuid::parse_str(&value()?)
                    .with_context(|| "--item-id must be a UUID".to_string())?;
                if !seen_item_ids.insert(parsed) {
                    bail!("--item-id repeats {parsed}");
                }
                args.item_ids.push(parsed);
            }
            "--page-size" => args.page_size = parse_i64("--page-size", &value()?)?,
            "--max-items" => args.max_items = parse_i64("--max-items", &value()?)?,
            "--max-artifact-bytes" => {
                args.max_artifact_bytes = parse_i64("--max-artifact-bytes", &value()?)?
            }
            "--max-total-bytes" => {
                args.max_total_bytes = parse_i64("--max-total-bytes", &value()?)?
            }
            "--delay-ms" => args.delay_ms = parse_u64("--delay-ms", &value()?)?,
            "--resume-cursor" => args.resume_cursor = Some(parse_cursor(&value()?)?),
            "--help" | "-h" => {
                args.help = true;
                return Ok(args);
            }
            other => bail!("unsupported argument {other}; run with --help for options"),
        }
    }
    validate_args(&args)?;
    Ok(args)
}

/// Boolean switches whose meaning is their presence. A supplied value, even
/// `false`, would otherwise read as "enabled" and silently authorize a write.
fn is_presence_only_flag(flag: &str) -> bool {
    matches!(
        flag,
        "--apply"
            | "--restore-inline"
            | "--acknowledge-reader-upgrade"
            | "--acknowledge-inline-growth"
            | "--help"
            | "-h"
    )
}

fn is_boolean_word(word: &str) -> bool {
    word.eq_ignore_ascii_case("true")
        || word.eq_ignore_ascii_case("false")
        || word.eq_ignore_ascii_case("yes")
        || word.eq_ignore_ascii_case("no")
}

fn parse_i64(flag: &str, value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .with_context(|| format!("{flag} must be an integer"))
}

fn parse_u64(flag: &str, value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("{flag} must be a non-negative integer"))
}

fn parse_cursor(value: &str) -> Result<ArtifactBackfillCursor> {
    let (item_id, kind) = value
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("--resume-cursor must be <uuid>:<download|sections>"))?;
    let item_id = Uuid::parse_str(item_id).context("--resume-cursor item id must be a UUID")?;
    let kind = match kind {
        "download" => TaskItemArtifactKind::Download,
        "sections" => TaskItemArtifactKind::Sections,
        other => bail!("--resume-cursor kind must be download or sections, got {other}"),
    };
    Ok(ArtifactBackfillCursor { item_id, kind })
}

fn validate_args(args: &ArtifactBackfillArgs) -> Result<()> {
    if args.page_size < 1 || args.page_size > MAX_PAGE_SIZE {
        bail!("--page-size must be between 1 and {MAX_PAGE_SIZE}");
    }
    if args.max_items < 1 || args.max_items > MAX_MAX_ITEMS {
        bail!("--max-items must be between 1 and {MAX_MAX_ITEMS}");
    }
    if args.max_artifact_bytes < 1 || args.max_artifact_bytes > MAX_ARTIFACT_BYTES_HARD_CAP {
        bail!("--max-artifact-bytes must be between 1 and {MAX_ARTIFACT_BYTES_HARD_CAP}");
    }
    if args.max_total_bytes < 1 || args.max_total_bytes > MAX_TOTAL_BYTES_HARD_CAP {
        bail!("--max-total-bytes must be between 1 and {MAX_TOTAL_BYTES_HARD_CAP}");
    }
    if args.delay_ms > MAX_DELAY_MS {
        bail!("--delay-ms must be between 0 and {MAX_DELAY_MS}");
    }
    if args.item_ids.len() > MAX_ITEM_IDS {
        bail!("--item-id may be repeated at most {MAX_ITEM_IDS} times");
    }
    match args.direction() {
        ArtifactDirection::Forward => {
            if args.acknowledge_inline_growth {
                bail!("--acknowledge-inline-growth is only valid with --restore-inline --apply");
            }
            if args.apply && !args.acknowledge_reader_upgrade {
                bail!("--apply requires --acknowledge-reader-upgrade");
            }
            if !args.apply && args.acknowledge_reader_upgrade {
                bail!("--acknowledge-reader-upgrade is only valid with --apply");
            }
        }
        ArtifactDirection::RestoreInline => {
            if args.acknowledge_reader_upgrade {
                bail!("--acknowledge-reader-upgrade is a forward-mode acknowledgement");
            }
            if args.apply && !args.acknowledge_inline_growth {
                bail!("--restore-inline --apply requires --acknowledge-inline-growth");
            }
            if !args.apply && args.acknowledge_inline_growth {
                bail!("--acknowledge-inline-growth is only valid with --restore-inline --apply");
            }
        }
    }
    if let Some(cursor) = args.resume_cursor
        && !args.kind.includes(cursor.kind)
    {
        bail!("--resume-cursor kind conflicts with --kind");
    }
    Ok(())
}

/// Per-kind counts plus the bytes those rows account for.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct ArtifactKindCounts {
    pub download: i64,
    pub sections: i64,
    pub bytes: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArtifactSample {
    pub item_id: Uuid,
    pub kind: &'static str,
}

/// Compact operator report. No payload text, filename, URL or digest value is
/// ever placed in it.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactBackfillReport {
    pub mode: &'static str,
    pub direction: &'static str,
    pub read_only: bool,
    pub artifact_schema: &'static str,
    pub externalized_state: &'static str,
    pub kind: &'static str,
    pub page_size: i64,
    pub max_items: i64,
    pub max_artifact_bytes: i64,
    pub max_total_bytes: i64,
    pub delay_ms: u64,
    pub input_validation: &'static str,
    pub scanned: i64,
    pub eligible: ArtifactKindCounts,
    pub applied: ArtifactKindCounts,
    pub restored: ArtifactKindCounts,
    pub skipped: BTreeMap<String, i64>,
    pub scan_complete: bool,
    pub resume_cursor: Option<String>,
    pub samples: Vec<ArtifactSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_artifact_references: Option<RemainingArtifactReferencesReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledgement: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Point-in-time remaining references. `snapshot` says explicitly that the
/// value is not a rollback certificate: an online application can create new
/// references after the query.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemainingArtifactReferencesReport {
    pub snapshot: &'static str,
    pub scope: &'static str,
    pub total: i64,
    pub download: i64,
    pub sections: i64,
    pub active: i64,
    pub terminal: i64,
    pub soft_deleted: i64,
}

impl From<RemainingArtifactReferences> for RemainingArtifactReferencesReport {
    fn from(references: RemainingArtifactReferences) -> Self {
        Self {
            snapshot: "point_in_time",
            scope: "all_references",
            total: references.total,
            download: references.download,
            sections: references.sections,
            active: references.active,
            terminal: references.terminal,
            soft_deleted: references.soft_deleted,
        }
    }
}

impl ArtifactBackfillReport {
    fn new(args: &ArtifactBackfillArgs, schema_present: bool) -> Self {
        Self {
            mode: if args.apply { "apply" } else { "preview" },
            direction: args.direction().as_str(),
            read_only: !args.apply,
            artifact_schema: if schema_present { "present" } else { "absent" },
            externalized_state: if schema_present { "known" } else { "unknown" },
            kind: args.kind.as_str(),
            page_size: args.page_size,
            max_items: args.max_items,
            max_artifact_bytes: args.max_artifact_bytes,
            max_total_bytes: args.max_total_bytes,
            delay_ms: args.delay_ms,
            input_validation: if args.apply {
                "performed"
            } else {
                "not_performed"
            },
            scanned: 0,
            eligible: ArtifactKindCounts::default(),
            applied: ArtifactKindCounts::default(),
            restored: ArtifactKindCounts::default(),
            skipped: BTreeMap::new(),
            scan_complete: true,
            resume_cursor: None,
            samples: Vec::new(),
            remaining_artifact_references: None,
            acknowledgement: args.apply.then_some(args.direction().acknowledgement()),
            error: None,
        }
    }

    fn restored(&mut self, kind: TaskItemArtifactKind, encoded_bytes: i64) {
        match kind {
            TaskItemArtifactKind::Download => self.restored.download += 1,
            TaskItemArtifactKind::Sections => self.restored.sections += 1,
        }
        self.restored.bytes = self.restored.bytes.saturating_add(encoded_bytes);
    }

    fn skip(&mut self, reason: &str) {
        *self.skipped.entry(reason.to_string()).or_insert(0) += 1;
    }

    fn eligible(&mut self, row: &ArtifactBackfillRow) {
        match row.kind {
            TaskItemArtifactKind::Download => self.eligible.download += 1,
            TaskItemArtifactKind::Sections => self.eligible.sections += 1,
        }
        self.eligible.bytes = self
            .eligible
            .bytes
            .saturating_add(row.encoded_len.unwrap_or(0));
        if self.samples.len() < SAMPLE_LIMIT {
            self.samples.push(ArtifactSample {
                item_id: row.item_id,
                kind: row.kind.kind(),
            });
        }
    }

    fn eligible_restore(&mut self, row: &ArtifactRestoreRow) {
        match row.kind {
            TaskItemArtifactKind::Download => self.eligible.download += 1,
            TaskItemArtifactKind::Sections => self.eligible.sections += 1,
        }
        let restored_len = match row.kind {
            TaskItemArtifactKind::Download => {
                base64_encoded_len(row.object_size_bytes).unwrap_or(row.object_size_bytes)
            }
            TaskItemArtifactKind::Sections => row.object_size_bytes,
        };
        self.eligible.bytes = self.eligible.bytes.saturating_add(restored_len);
        if self.samples.len() < SAMPLE_LIMIT {
            self.samples.push(ArtifactSample {
                item_id: row.item_id,
                kind: row.kind.kind(),
            });
        }
    }

    fn applied(&mut self, kind: TaskItemArtifactKind, bytes: i64) {
        match kind {
            TaskItemArtifactKind::Download => self.applied.download += 1,
            TaskItemArtifactKind::Sections => self.applied.sections += 1,
        }
        self.applied.bytes = self.applied.bytes.saturating_add(bytes);
    }
}

/// Why one metadata row is not eligible for apply.
fn classify_row(row: &ArtifactBackfillRow, max_artifact_bytes: i64) -> Option<&'static str> {
    if row.task_deleted {
        return Some("deleted_parent");
    }
    if !is_terminal_status(&row.task_status) {
        return Some("parent_not_terminal");
    }
    if row.lease_active {
        return Some("active_item");
    }
    if row.group_id.is_none() {
        return Some("no_group");
    }
    let Some(field_type) = row.field_type.as_deref() else {
        return Some(if row.artifact_present {
            "already_externalized"
        } else {
            "no_inline"
        });
    };
    let expected = match row.kind {
        TaskItemArtifactKind::Download => "string",
        TaskItemArtifactKind::Sections => "array",
    };
    if field_type != expected {
        return Some("invalid_shape");
    }
    if row.encoded_len.is_none_or(|len| len > max_artifact_bytes) {
        return Some("oversize");
    }
    if row.kind == TaskItemArtifactKind::Download && !row.declared_sha256_present {
        return Some("missing_declared_sha");
    }
    if row.kind == TaskItemArtifactKind::Download && !row.declared_sha256_valid {
        return Some("invalid_declared_sha");
    }
    None
}

/// Map a refusal word produced by the capped fetch statement onto a static
/// skip class, so an unknown word can never be mistaken for success.
fn known_skip_reason(reason: &str) -> &'static str {
    match reason {
        "no_inline" => "no_inline",
        "invalid_shape" => "invalid_shape",
        "oversize" => "oversize",
        _ => "race_changed",
    }
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

async fn next_page(
    db: &Database,
    args: &ArtifactBackfillArgs,
    schema_present: bool,
    cursor: Option<ArtifactBackfillCursor>,
    limit: i64,
) -> Result<Vec<ArtifactBackfillRow>> {
    let item_ids = (!args.item_ids.is_empty()).then_some(args.item_ids.as_slice());
    if schema_present {
        db.list_terminal_artifact_states(args.kind.as_str(), cursor, limit, item_ids)
            .await
    } else {
        db.list_inline_artifact_candidates(args.kind.as_str(), cursor, limit, item_ids)
            .await
    }
}

/// Run the default, read-only metadata preview.
pub async fn run_preview(
    db: &Database,
    args: &ArtifactBackfillArgs,
) -> Result<ArtifactBackfillReport> {
    if args.apply {
        bail!("run_preview cannot run an apply");
    }
    let schema_present = db.task_item_artifact_schema_present().await?;
    let mut report = ArtifactBackfillReport::new(args, schema_present);
    let mut cursor = args.resume_cursor;
    let mut scanned = 0_i64;
    let mut complete = true;

    loop {
        if scanned >= args.max_items {
            complete = false;
            break;
        }
        let page = next_page(db, args, schema_present, cursor, args.page_size + 1).await?;
        let has_extra = page.len() as i64 > args.page_size;
        let page: Vec<_> = page.into_iter().take(args.page_size as usize).collect();
        if page.is_empty() {
            break;
        }
        let mut stopped_early = false;
        for row in &page {
            if scanned >= args.max_items {
                stopped_early = true;
                break;
            }
            scanned += 1;
            report.scanned += 1;
            cursor = Some(ArtifactBackfillCursor {
                item_id: row.item_id,
                kind: row.kind,
            });
            match classify_row(row, args.max_artifact_bytes) {
                Some(reason) => report.skip(reason),
                None => report.eligible(row),
            }
        }
        if stopped_early || has_extra {
            complete = false;
            break;
        }
        if page.len() as i64 == args.page_size {
            continue;
        }
        break;
    }

    report.scan_complete = complete;
    report.resume_cursor = if complete {
        None
    } else {
        cursor.map(|cursor| format!("{}:{}", cursor.item_id, cursor.kind.kind()))
    };
    Ok(report)
}

/// Base64 length of `raw_bytes` without allocating the encoding.
fn base64_encoded_len(raw_bytes: i64) -> Option<i64> {
    if raw_bytes < 0 {
        return None;
    }
    Some(4 * raw_bytes.saturating_add(2).div_euclid(3))
}

/// Why one existing reference is not eligible for the inverse direction.
fn classify_restore_row(
    row: &ArtifactRestoreRow,
    max_artifact_bytes: i64,
    active_backend: &str,
) -> Option<&'static str> {
    if row.task_deleted {
        return Some("deleted_parent");
    }
    if !is_terminal_status(&row.task_status) {
        return Some("parent_not_terminal");
    }
    if row.lease_active {
        return Some("active_item");
    }
    match row.task_group_id {
        None => return Some("no_group"),
        Some(group_id) if group_id != row.object_group_id => return Some("group_mismatch"),
        Some(_) => {}
    }
    if row.object_storage_backend != active_backend {
        return Some("inactive_backend");
    }
    if row.inline_type.is_some() {
        return Some("inline_present");
    }
    if row.kind == TaskItemArtifactKind::Download {
        if row.download_metadata_type.as_deref() != Some("object") {
            return Some("missing_download_metadata");
        }
        match row.declared_sha256.as_deref() {
            None => return Some("missing_declared_sha"),
            Some(declared) if !is_hex_sha256(declared) => return Some("invalid_declared_sha"),
            Some(_) => {}
        }
    }
    let encoded_len = match row.kind {
        TaskItemArtifactKind::Download => {
            let Some(encoded_len) = base64_encoded_len(row.object_size_bytes) else {
                return Some("invalid_registry_size");
            };
            encoded_len
        }
        TaskItemArtifactKind::Sections => row.object_size_bytes,
    };
    if encoded_len > max_artifact_bytes {
        return Some("oversize");
    }
    None
}

async fn next_restore_page(
    db: &Database,
    args: &ArtifactBackfillArgs,
    cursor: Option<ArtifactBackfillCursor>,
    limit: i64,
) -> Result<Vec<ArtifactRestoreRow>> {
    let item_ids = (!args.item_ids.is_empty()).then_some(args.item_ids.as_slice());
    db.list_artifact_restore_candidates(args.kind.as_str(), cursor, limit, item_ids)
        .await
}

/// Run the read-only metadata preview for the inverse direction.
///
/// The artifact relation and the 975 ledger entry are required before any scan:
/// without them there is nothing to restore, and a zero result would be a false
/// "complete" claim.
pub async fn run_restore_preview(
    db: &Database,
    args: &ArtifactBackfillArgs,
    active_backend: &str,
) -> Result<ArtifactBackfillReport> {
    if args.apply || !args.restore_inline {
        bail!("run_restore_preview requires preview restore-inline flags");
    }
    validate_restore_source(db).await?;
    let mut report = ArtifactBackfillReport::new(args, true);
    let mut cursor = args.resume_cursor;
    let mut scanned = 0_i64;
    let mut complete = true;

    loop {
        if scanned >= args.max_items {
            complete = false;
            break;
        }
        let page = next_restore_page(db, args, cursor, args.page_size + 1).await?;
        let has_extra = page.len() as i64 > args.page_size;
        let page: Vec<_> = page.into_iter().take(args.page_size as usize).collect();
        if page.is_empty() {
            break;
        }
        let mut stopped_early = false;
        for row in &page {
            if scanned >= args.max_items {
                stopped_early = true;
                break;
            }
            scanned += 1;
            report.scanned += 1;
            cursor = Some(ArtifactBackfillCursor {
                item_id: row.item_id,
                kind: row.kind,
            });
            match classify_restore_row(row, args.max_artifact_bytes, active_backend) {
                Some(reason) => report.skip(reason),
                None => report.eligible_restore(row),
            }
        }
        if stopped_early || has_extra {
            complete = false;
            break;
        }
        if page.len() as i64 == args.page_size {
            continue;
        }
        break;
    }

    report.scan_complete = complete;
    report.resume_cursor = if complete {
        None
    } else {
        cursor.map(|cursor| format!("{}:{}", cursor.item_id, cursor.kind.kind()))
    };
    Ok(report)
}

/// Require the artifact relation and the exact applied 975 ledger entry.
pub async fn validate_restore_source(db: &Database) -> Result<()> {
    if !db.task_item_artifact_schema_present().await? {
        bail!(
            "restore-inline is not available: the artifact relation \
             context69.task_item_artifacts does not exist, so nothing can be restored"
        );
    }
    let applied = db
        .applied_migration_checksum(ARTIFACT_MIGRATION_VERSION)
        .await
        .context("failed to read the migration ledger")?;
    validate_applied_migration(applied, &compiled_migration_checksum()?)
}

/// Run the explicitly acknowledged inverse apply.
///
/// The returned report may carry `error`; in that case the caller must exit
/// non-zero. Skips and races are reported separately and are not errors.
pub async fn run_restore_apply(
    db: &Database,
    library: &LibraryService,
    args: &ArtifactBackfillArgs,
) -> Result<ArtifactBackfillReport> {
    if !args.apply || !args.restore_inline || !args.acknowledge_inline_growth {
        bail!("restore apply requires --restore-inline --apply --acknowledge-inline-growth");
    }
    validate_restore_source(db).await?;
    let active_backend = library.maintenance_storage_backend().to_string();
    let mut report = ArtifactBackfillReport::new(args, true);
    let mut cursor = args.resume_cursor;
    let mut scanned = 0_i64;
    let mut total_bytes = 0_i64;
    let mut complete = true;

    'scan: loop {
        if scanned >= args.max_items {
            complete = false;
            break;
        }
        let page = next_restore_page(db, args, cursor, args.page_size + 1).await?;
        let has_extra = page.len() as i64 > args.page_size;
        let page: Vec<_> = page.into_iter().take(args.page_size as usize).collect();
        if page.is_empty() {
            break;
        }
        for row in &page {
            if scanned >= args.max_items {
                complete = false;
                break 'scan;
            }
            scanned += 1;
            report.scanned += 1;
            cursor = Some(ArtifactBackfillCursor {
                item_id: row.item_id,
                kind: row.kind,
            });
            let Some(reason) = classify_restore_row(row, args.max_artifact_bytes, &active_backend)
            else {
                match restore_one(db, library, row, args, &mut total_bytes).await {
                    Ok(ApplyOne::Applied { bytes }) => report.restored(row.kind, bytes),
                    Ok(ApplyOne::Skipped(reason)) => {
                        if reason == "total_budget" {
                            report.skip(reason);
                            complete = false;
                            break 'scan;
                        }
                        report.skip(reason);
                    }
                    Err(_error) => {
                        report.error = Some(
                            "restore stopped on an unexpected storage or database error; the failing \
                             item's reference and inline field are unchanged"
                                .to_string(),
                        );
                        report.scan_complete = false;
                        report.resume_cursor = cursor
                            .map(|cursor| format!("{}:{}", cursor.item_id, cursor.kind.kind()));
                        return Ok(report);
                    }
                }
                if args.delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
                }
                continue;
            };
            report.skip(reason);
        }
        if has_extra {
            complete = false;
            break;
        }
        if (page.len() as i64) < args.page_size {
            break;
        }
    }

    report.scan_complete = complete;
    report.resume_cursor = if complete {
        None
    } else {
        cursor.map(|cursor| format!("{}:{}", cursor.item_id, cursor.kind.kind()))
    };
    report.remaining_artifact_references = Some(db.remaining_artifact_references().await?.into());
    Ok(report)
}

/// Restore one verified reference into its missing inline field.
async fn restore_one(
    db: &Database,
    library: &LibraryService,
    row: &ArtifactRestoreRow,
    args: &ArtifactBackfillArgs,
    total_bytes: &mut i64,
) -> Result<ApplyOne> {
    let Some(reference) = library
        .load_task_item_artifact(row.item_id, row.kind)
        .await?
    else {
        return Ok(ApplyOne::Skipped("changed_ref"));
    };
    let same_identity = reference.storage_object_id == row.storage_object_id
        && reference.kind == row.kind.kind()
        && reference.encoding == row.encoding
        && reference.group_id == row.object_group_id
        && reference.sha256 == row.object_sha256
        && reference.size_bytes == row.object_size_bytes
        && reference.storage_backend == row.object_storage_backend
        && reference.resolved_kind() == Some(row.kind);
    if !same_identity {
        return Ok(ApplyOne::Skipped("changed_ref"));
    }

    let encoded_len = match row.kind {
        TaskItemArtifactKind::Download => match base64_encoded_len(reference.size_bytes) {
            Some(encoded_len) => encoded_len,
            None => return Ok(ApplyOne::Skipped("invalid_registry_size")),
        },
        TaskItemArtifactKind::Sections => reference.size_bytes,
    };
    if encoded_len > args.max_artifact_bytes {
        return Ok(ApplyOne::Skipped("oversize"));
    }
    let expected_total = total_bytes.saturating_add(encoded_len);
    if expected_total > args.max_total_bytes {
        return Ok(ApplyOne::Skipped("total_budget"));
    }

    let cap = args.max_artifact_bytes as usize;
    let mut download_base64 = None;
    let mut sections_json = None;
    match row.kind {
        TaskItemArtifactKind::Download => {
            let declared = row.declared_sha256.as_deref().unwrap_or_default();
            if !declared.eq_ignore_ascii_case(&reference.sha256) {
                return Ok(ApplyOne::Skipped("sha_mismatch"));
            }
            let Some(bytes) = library
                .read_task_item_artifact(row.item_id, row.kind, cap)
                .await?
            else {
                return Ok(ApplyOne::Skipped("changed_ref"));
            };
            download_base64 = Some(STANDARD.encode(&bytes));
        }
        TaskItemArtifactKind::Sections => {
            let Some(bytes) = library
                .read_task_item_artifact(row.item_id, row.kind, cap)
                .await?
            else {
                return Ok(ApplyOne::Skipped("changed_ref"));
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return Ok(ApplyOne::Skipped("invalid_sections"));
            };
            if serde_json::from_value::<Vec<IngestSection>>(value.clone()).is_err() {
                return Ok(ApplyOne::Skipped("invalid_sections"));
            }
            sections_json = Some(value);
        }
    }

    let restored = db
        .restore_task_item_artifact_inline(RestoreTaskItemArtifactInlineRequest {
            item_id: row.item_id,
            kind: row.kind,
            storage_object_id: reference.storage_object_id,
            sha256: &reference.sha256,
            size_bytes: reference.size_bytes,
            storage_backend: &reference.storage_backend,
            encoding: &reference.encoding,
            download_base64: download_base64.as_deref(),
            sections_json: sections_json.clone(),
        })
        .await?;
    if !restored {
        return Ok(ApplyOne::Skipped("race_changed"));
    }
    *total_bytes = expected_total;
    Ok(ApplyOne::Applied { bytes: encoded_len })
}

/// Validate that this binary may apply before any storage is touched.
pub async fn validate_apply(db: &Database) -> Result<()> {
    if !db.task_item_artifact_schema_present().await? {
        bail!(
            "the artifact relation context69.task_item_artifacts does not exist; apply the \
             compiled #975 migration {ARTIFACT_MIGRATION_VERSION} before running --apply"
        );
    }
    let applied = db
        .applied_migration_checksum(ARTIFACT_MIGRATION_VERSION)
        .await
        .context("failed to read the migration ledger")?;
    validate_applied_migration(applied, &compiled_migration_checksum()?)
}

/// The fail-closed ledger gate, independent of the database connection so a
/// refused state is testable without touching the ledger.
fn validate_applied_migration(applied: Option<(Vec<u8>, bool)>, compiled: &[u8]) -> Result<()> {
    let Some((checksum, success)) = applied else {
        bail!(
            "migration {ARTIFACT_MIGRATION_VERSION} is not recorded as applied; apply it before \
             running an artifact apply"
        );
    };
    if !success {
        bail!("migration {ARTIFACT_MIGRATION_VERSION} did not finish successfully");
    }
    if checksum != compiled {
        bail!(
            "migration {ARTIFACT_MIGRATION_VERSION} was applied with a different checksum than this \
             binary; refusing to modify inline data"
        );
    }
    Ok(())
}

fn compiled_migration_checksum() -> Result<Vec<u8>> {
    let migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| migration.version == ARTIFACT_MIGRATION_VERSION)
        .with_context(|| format!("binary does not embed migration {ARTIFACT_MIGRATION_VERSION}"))?;
    Ok(migration.checksum.to_vec())
}

/// The storage backend a deployment configuration selects. Used by the
/// read-only restore preview, which has no library handle.
pub fn configured_storage_backend(config: &Config) -> &'static str {
    if config.file_library.s3.is_some() {
        "s3"
    } else {
        "local"
    }
}

/// Run one maintenance invocation from CLI arguments.
///
/// This is the single implementation behind both `task_artifact_backfill` and
/// `context69 task-artifacts`: parsing, validation, help, URL resolution and
/// every mode live here so no second implementation can drift. The function
/// returns instead of exiting, so the caller owns the process exit status; a
/// batch-level error is returned after the report was printed.
pub async fn run_cli<I, S>(raw: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = parse_args(raw)?;
    if args.help {
        print!("{}", help_text());
        return Ok(());
    }

    db_init::load_dotenv_if_exists(".env").context("load repository .env for the operator tool")?;
    let resolution = db_init::resolve_database_url(None, &[DATABASE_URL_ENV_VAR], || Ok(None))
        .context("missing database URL; set DATABASE_URL or the repository .env")?;

    let db = Database::connect_operator(&resolution.database_url, !args.apply)
        .await
        .context("connect the operator database pool")?;

    if args.restore_inline {
        validate_restore_source(&db).await?;
        let mut config = Config::load()?;
        let report = if args.apply {
            let library = build_maintenance_library(&db, &mut config).await?;
            run_restore_apply(&db, &library, &args).await?
        } else {
            app::hydrate_runtime_settings_for_maintenance(&db, &mut config).await?;
            let backend = configured_storage_backend(&config);
            run_restore_preview(&db, &args, backend).await?
        };
        return finish_report(&report);
    }

    if !args.apply {
        let report = run_preview(&db, &args).await?;
        return finish_report(&report);
    }

    validate_apply(&db).await?;
    let mut config = Config::load()?;
    let library = build_maintenance_library(&db, &mut config).await?;
    let report = run_apply(&db, &library, &args).await?;
    finish_report(&report)
}

/// Print one report and turn a batch error into a returned failure so the
/// caller exits non-zero without starting any other mode.
fn finish_report(report: &ArtifactBackfillReport) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(report)?);
    if report.error.is_some() {
        bail!(
            "artifact maintenance stopped on an error; the batch report above names the failing \
             item's state"
        );
    }
    Ok(())
}

/// Build the narrow maintenance library service: runtime settings resolved
/// read-only, no bootstrap, no settings write, no workers.
pub async fn build_maintenance_library(
    db: &Database,
    config: &mut Config,
) -> Result<LibraryService> {
    let _: Option<StoredRuntimeSettings> =
        app::hydrate_runtime_settings_for_maintenance(db, config).await?;
    let secrets = secret_store::build(db, config)?;
    let settings = SettingsService::with_secrets(db.clone(), secrets);
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    LibraryService::new(
        db.clone(),
        EmbeddingRuntime::default(),
        None,
        LibraryServiceConfig {
            chunking: config.chunking.clone(),
            file_library: config.file_library.clone(),
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "maintenance-backfill".to_string(),
        },
        settings,
        translation,
    )
    .await
}

struct NoopCallbacks;

#[async_trait::async_trait]
impl context69_translation::TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: context69_translation::TranslationPublication<'_>,
    ) -> Result<Vec<context69_translation::TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl context69_translation::TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// Run the explicitly acknowledged apply.
///
/// The returned report may carry `error`; in that case the caller must exit
/// non-zero. Skips and races are reported separately and are not errors.
pub async fn run_apply(
    db: &Database,
    library: &LibraryService,
    args: &ArtifactBackfillArgs,
) -> Result<ArtifactBackfillReport> {
    if !args.apply || !args.acknowledge_reader_upgrade {
        bail!("apply requires --apply and --acknowledge-reader-upgrade");
    }
    if !db.task_item_artifact_schema_present().await? {
        bail!("artifact relation disappeared after validation");
    }
    let mut report = ArtifactBackfillReport::new(args, true);
    let mut cursor = args.resume_cursor;
    let mut scanned = 0_i64;
    let mut total_bytes = 0_i64;
    let mut complete = true;

    'scan: loop {
        if scanned >= args.max_items {
            complete = false;
            break;
        }
        let page = next_page(db, args, true, cursor, args.page_size + 1).await?;
        let has_extra = page.len() as i64 > args.page_size;
        let page: Vec<_> = page.into_iter().take(args.page_size as usize).collect();
        if page.is_empty() {
            break;
        }
        for row in &page {
            if scanned >= args.max_items {
                complete = false;
                break 'scan;
            }
            scanned += 1;
            report.scanned += 1;
            cursor = Some(ArtifactBackfillCursor {
                item_id: row.item_id,
                kind: row.kind,
            });
            let Some(reason) = classify_row(row, args.max_artifact_bytes) else {
                match apply_one(db, library, row, args, &mut total_bytes).await {
                    Ok(ApplyOne::Applied { bytes }) => {
                        report.applied(row.kind, bytes);
                    }
                    Ok(ApplyOne::Skipped(reason)) => {
                        if reason == "total_budget" {
                            report.skip(reason);
                            complete = false;
                            break 'scan;
                        }
                        report.skip(reason);
                    }
                    Err(_error) => {
                        report.error = Some(
                            "apply stopped on an unexpected storage or database error; the failing \
                             item's inline field is untouched"
                                .to_string(),
                        );
                        report.scan_complete = false;
                        report.resume_cursor = cursor
                            .map(|cursor| format!("{}:{}", cursor.item_id, cursor.kind.kind()));
                        return Ok(report);
                    }
                }
                if args.delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
                }
                continue;
            };
            report.skip(reason);
        }
        if has_extra {
            complete = false;
            break;
        }
        if (page.len() as i64) < args.page_size {
            break;
        }
    }

    report.scan_complete = complete;
    report.resume_cursor = if complete {
        None
    } else {
        cursor.map(|cursor| format!("{}:{}", cursor.item_id, cursor.kind.kind()))
    };
    Ok(report)
}

enum ApplyOne {
    Applied { bytes: i64 },
    Skipped(&'static str),
}

async fn apply_one(
    db: &Database,
    library: &LibraryService,
    row: &ArtifactBackfillRow,
    args: &ArtifactBackfillArgs,
    total_bytes: &mut i64,
) -> Result<ApplyOne> {
    let Some(field) = db
        .fetch_artifact_backfill_field(row.item_id, row.kind, args.max_artifact_bytes)
        .await
        .with_context(|| format!("failed to fetch the inline field for item {}", row.item_id))?
    else {
        return Ok(ApplyOne::Skipped("no_inline"));
    };
    // The fetch statement already rechecked the owner state, shape and current
    // length in one snapshot; a changed or oversized field arrives with no text.
    if let Some(reason) = field.skip_reason.as_deref() {
        return Ok(ApplyOne::Skipped(known_skip_reason(reason)));
    }
    if field.field_text.is_none() {
        return Ok(ApplyOne::Skipped("race_changed"));
    }
    if !is_terminal_status(&field.item_status)
        || !is_terminal_status(&field.task_status)
        || field.task_deleted
        || field.lease_active
    {
        return Ok(ApplyOne::Skipped("race_changed"));
    }
    let Some(group_id) = field.group_id else {
        return Ok(ApplyOne::Skipped("no_group"));
    };
    let Some(field_text) = field.field_text else {
        return Ok(ApplyOne::Skipped("no_inline"));
    };
    if field_text.len() as i64 > args.max_artifact_bytes {
        return Ok(ApplyOne::Skipped("oversize"));
    }
    let fingerprint = sha256_hex(field_text.as_bytes());
    let bytes = match row.kind {
        TaskItemArtifactKind::Download => {
            let Some(declared) = field.declared_sha256.as_deref() else {
                return Ok(ApplyOne::Skipped("missing_declared_sha"));
            };
            if !is_hex_sha256(declared) {
                return Ok(ApplyOne::Skipped("invalid_declared_sha"));
            }
            let Some(decoded) =
                decode_base64_bounded(&field_text, args.max_artifact_bytes as usize)?
            else {
                return Ok(ApplyOne::Skipped("oversize"));
            };
            let actual = sha256_hex(&decoded);
            if !actual.eq_ignore_ascii_case(declared) {
                return Ok(ApplyOne::Skipped("sha_mismatch"));
            }
            decoded
        }
        TaskItemArtifactKind::Sections => {
            if serde_json::from_str::<Vec<IngestSection>>(&field_text).is_err() {
                return Ok(ApplyOne::Skipped("invalid_sections"));
            }
            field_text.into_bytes()
        }
    };
    if bytes.len() as i64 > args.max_artifact_bytes {
        return Ok(ApplyOne::Skipped("oversize"));
    }
    let byte_len = bytes.len() as i64;
    let expected_total = total_bytes.saturating_add(byte_len);
    if expected_total > args.max_total_bytes {
        return Ok(ApplyOne::Skipped("total_budget"));
    }
    let staged = library
        .stage_maintenance_task_artifact(group_id, Bytes::from(bytes))
        .await?;
    let attached = db
        .attach_terminal_task_item_artifact(AttachTerminalTaskItemArtifactRequest {
            item_id: row.item_id,
            kind: row.kind,
            storage_object_id: staged.object_id,
            sha256: &staged.sha256,
            size_bytes: staged.size_bytes,
            storage_backend: library.maintenance_storage_backend(),
            field_fingerprint: &fingerprint,
        })
        .await?;
    if !attached {
        library.discard_maintenance_task_artifact(&staged).await;
        return Ok(ApplyOne::Skipped("field_changed"));
    }
    library.promote_maintenance_task_artifact(&staged).await?;
    *total_bytes = expected_total;
    Ok(ApplyOne::Applied { bytes: byte_len })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_hex_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Decode standard base64 in bounded windows, refusing to allocate past the
/// caller's cap. `Ok(None)` means the decoded size exceeded the cap.
fn decode_base64_bounded(encoded: &str, max_bytes: usize) -> Result<Option<Vec<u8>>> {
    let encoded = encoded.trim().as_bytes();
    if !encoded.len().is_multiple_of(4) {
        bail!("invalid base64 input length");
    }
    let mut decoded = Vec::with_capacity((encoded.len() / 4).saturating_mul(3));
    for window in encoded.chunks(BASE64_CHUNK_BYTES) {
        let start = decoded.len();
        decoded.resize(start + (window.len() / 4) * 3 + 3, 0);
        let written = STANDARD
            .decode_slice(window, &mut decoded[start..])
            .map_err(|error| anyhow::anyhow!("invalid base64 input: {error}"))?;
        decoded.truncate(start + written);
        if decoded.len() > max_bytes {
            return Ok(None);
        }
    }
    Ok(Some(decoded))
}

/// Public help text for the operator binary.
pub fn help_text() -> &'static str {
    concat!(
        "task_artifact_backfill (issue #975)\n",
        "\n",
        "Forward mode moves terminal inline download/sections bytes into the\n",
        "content-addressed artifact store. It previews by default; nothing is\n",
        "uploaded or written without --apply. Restore mode is the bounded inverse:\n",
        "it copies verified artifact bytes back into a missing inline field and\n",
        "removes exactly that reference. See docs/task-artifacts.md.\n",
        "\n",
        "Usage: context69 task-artifacts [OPTIONS]\n",
        "       task_artifact_backfill [OPTIONS]   (development binary)\n",
        "  --apply                          forward apply; requires the reader ack\n",
        "  --acknowledge-reader-upgrade     required with forward --apply\n",
        "  --restore-inline                 inverse direction (preview by default)\n",
        "  --acknowledge-inline-growth      required with --restore-inline --apply\n",
        "  --kind <download|sections|all>   default all\n",
        "  --item-id <uuid>                 repeatable, bounded selection\n",
        "  --page-size <1-1000>             default 100\n",
        "  --max-items <1-1000000>          default 500\n",
        "  --max-artifact-bytes <n>         default/hard cap 256 MiB\n",
        "  --max-total-bytes <n>            default 512 MiB\n",
        "  --delay-ms <0-60000>             default 100\n",
        "  --resume-cursor <uuid:kind>      resume after the last scanned pair\n",
        "  --help\n",
        "\n",
        "The database URL comes only from DATABASE_URL / the repository .env, never\n",
        "from argv. Output is JSON metadata only: no content, filenames, URLs or\n",
        "digest values. Restoring inline bytes can grow PostgreSQL and does not make\n",
        "an older application version safe to deploy.\n",
        "\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn defaults_are_preview_only_with_fixed_limits() {
        let args = parse_args(base_args()).expect("defaults parse");
        assert!(!args.apply);
        assert!(!args.acknowledge_reader_upgrade);
        assert_eq!(args.kind, ArtifactKindFilter::All);
        assert_eq!(args.page_size, 100);
        assert_eq!(args.max_items, 500);
        assert_eq!(args.max_artifact_bytes, 256 * 1024 * 1024);
        assert_eq!(args.max_total_bytes, 512 * 1024 * 1024);
        assert_eq!(args.delay_ms, 100);
    }

    #[test]
    fn apply_without_acknowledgement_fails_before_connection() {
        let error = parse_args(["--apply"]).expect_err("ack is mandatory");
        assert!(error.to_string().contains("--acknowledge-reader-upgrade"));
    }

    #[test]
    fn zero_and_out_of_bound_limits_are_rejected() {
        for raw in [
            vec!["--page-size", "0"],
            vec!["--page-size", "1001"],
            vec!["--max-items", "0"],
            vec!["--max-artifact-bytes", "268435457"],
            vec!["--max-total-bytes", "0"],
            vec!["--delay-ms", "60001"],
            vec!["--kind", "other"],
            vec!["--item-id", "not-a-uuid"],
            vec!["--resume-cursor", "not-a-cursor"],
            vec!["--acknowledge-reader-upgrade"],
            vec!["--unknown"],
        ] {
            assert!(parse_args(raw).is_err());
        }
    }

    #[test]
    fn repeated_item_ids_are_rejected() {
        let id = Uuid::new_v4().to_string();
        let error = parse_args(["--item-id", &id, "--item-id", &id]).expect_err("duplicate");
        assert!(error.to_string().contains("repeats"));
    }

    #[test]
    fn resume_cursor_conflicting_with_kind_is_rejected() {
        let cursor = format!("{}:sections", Uuid::new_v4());
        assert!(parse_args(["--kind", "download", "--resume-cursor", &cursor]).is_err());
    }

    #[test]
    fn invalid_base64_is_refused() {
        assert!(decode_base64_bounded("not base64!", 1024).is_err());
    }

    #[test]
    fn decoded_bytes_over_the_cap_are_not_returned() {
        let encoded = STANDARD.encode([0_u8; 16]);
        assert_eq!(
            decode_base64_bounded(&encoded, 8).expect("valid base64"),
            None
        );
        assert_eq!(
            decode_base64_bounded(&encoded, 16).expect("valid base64"),
            Some(vec![0_u8; 16])
        );
    }

    #[test]
    fn fingerprint_is_sha256_of_the_utf8_field_text() {
        assert_eq!(
            sha256_hex(b"[]"),
            "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
        );
    }

    fn metadata_row(kind: TaskItemArtifactKind) -> ArtifactBackfillRow {
        ArtifactBackfillRow {
            item_id: Uuid::nil(),
            kind,
            task_id: Uuid::nil(),
            group_id: Some(1),
            item_status: "succeeded".to_string(),
            task_status: "succeeded".to_string(),
            task_deleted: false,
            lease_active: false,
            field_type: Some("string".to_string()),
            encoded_len: Some(4),
            declared_sha256_present: true,
            declared_sha256_valid: true,
            artifact_present: false,
        }
    }

    #[test]
    fn classification_reports_explicit_skip_classes() {
        let mut row = metadata_row(TaskItemArtifactKind::Download);
        assert_eq!(classify_row(&row, 1024), None);
        row.lease_active = true;
        assert_eq!(classify_row(&row, 1024), Some("active_item"));
        row.lease_active = false;
        row.group_id = None;
        assert_eq!(classify_row(&row, 1024), Some("no_group"));
        row.group_id = Some(1);
        row.task_deleted = true;
        assert_eq!(classify_row(&row, 1024), Some("deleted_parent"));
        row.task_deleted = false;
        row.task_status = "running".to_string();
        assert_eq!(classify_row(&row, 1024), Some("parent_not_terminal"));
        row.task_status = "succeeded".to_string();
        row.field_type = None;
        assert_eq!(classify_row(&row, 1024), Some("no_inline"));
        row.artifact_present = true;
        assert_eq!(classify_row(&row, 1024), Some("already_externalized"));
        row.artifact_present = false;
        row.field_type = Some("object".to_string());
        assert_eq!(classify_row(&row, 1024), Some("invalid_shape"));
        row.field_type = Some("string".to_string());
        row.encoded_len = Some(2048);
        assert_eq!(classify_row(&row, 1024), Some("oversize"));
        row.encoded_len = Some(4);
        row.declared_sha256_present = false;
        assert_eq!(classify_row(&row, 1024), Some("missing_declared_sha"));
        row.declared_sha256_present = true;
        row.declared_sha256_valid = false;
        assert_eq!(classify_row(&row, 1024), Some("invalid_declared_sha"));
    }

    #[test]
    fn sections_kind_does_not_require_a_declared_digest() {
        let mut row = metadata_row(TaskItemArtifactKind::Sections);
        row.field_type = Some("array".to_string());
        row.declared_sha256_present = false;
        row.declared_sha256_valid = false;
        assert_eq!(classify_row(&row, 1024), None);
    }

    #[test]
    fn apply_fails_closed_on_a_missing_or_mismatched_migration_ledger_entry() {
        let compiled = compiled_migration_checksum().expect("compiled checksum");
        assert_eq!(compiled.len(), 48);

        let missing = validate_applied_migration(None, &compiled)
            .expect_err("a missing ledger row must refuse apply");
        assert!(missing.to_string().contains("not recorded as applied"));

        let unfinished = validate_applied_migration(Some((compiled.clone(), false)), &compiled)
            .expect_err("an unsuccessful migration must refuse apply");
        assert!(
            unfinished
                .to_string()
                .contains("did not finish successfully")
        );

        let mismatched = validate_applied_migration(Some((vec![0_u8; 48], true)), &compiled)
            .expect_err("a different applied checksum must refuse apply");
        assert!(mismatched.to_string().contains("different checksum"));

        validate_applied_migration(Some((compiled.clone(), true)), &compiled)
            .expect("the exact applied checksum admits apply");
    }

    #[test]
    fn preview_without_the_artifact_relation_uses_the_table_free_query() {
        let inline =
            include_str!("../../sql/db/tasks/artifact_backfill/list_inline_candidates.sql");
        let terminal =
            include_str!("../../sql/db/tasks/artifact_backfill/list_terminal_artifact_states.sql");
        assert!(
            !inline.contains("FROM context69.task_item_artifacts"),
            "the pre-artifact preview query must not reference the missing relation"
        );
        assert!(terminal.contains("task_item_artifacts"));
    }

    #[test]
    fn restore_mode_flag_and_acknowledgement_matrix() {
        assert!(parse_args(["--restore-inline"]).is_ok());
        assert!(parse_args(["--restore-inline", "--apply"]).is_err());
        assert!(parse_args(["--restore-inline", "--apply", "--acknowledge-inline-growth"]).is_ok());
        assert!(
            parse_args([
                "--restore-inline",
                "--apply",
                "--acknowledge-inline-growth",
                "--acknowledge-reader-upgrade"
            ])
            .is_err(),
            "the forward reader acknowledgement is not a restore acknowledgement"
        );
        assert!(
            parse_args([
                "--apply",
                "--acknowledge-reader-upgrade",
                "--acknowledge-inline-growth"
            ])
            .is_err(),
            "the growth acknowledgement is not a forward acknowledgement"
        );
        assert!(parse_args(["--restore-inline", "--acknowledge-inline-growth"]).is_err());
    }

    #[test]
    fn presence_only_flags_reject_values_in_both_directions() {
        for flag in [
            "--apply",
            "--restore-inline",
            "--acknowledge-reader-upgrade",
            "--acknowledge-inline-growth",
            "--help",
        ] {
            for value in ["true", "false", "1", "0"] {
                let inline = format!("{flag}={value}");
                let error = parse_args([inline.as_str()])
                    .expect_err("a value on a presence-only flag must fail");
                assert!(
                    error.to_string().contains("presence-only"),
                    "{flag}={value} gave: {error}"
                );
            }
            for word in ["true", "false"] {
                let error =
                    parse_args([flag, word]).expect_err("a trailing boolean word must fail");
                assert!(
                    error.to_string().contains("presence-only"),
                    "{flag} {word} gave: {error}"
                );
            }
        }
        // A bare occurrence still parses.
        assert!(parse_args(["--restore-inline"]).is_ok());
        assert!(parse_args(["--apply", "--acknowledge-reader-upgrade"]).is_ok());
        assert!(parse_args(["--restore-inline", "--apply", "--acknowledge-inline-growth"]).is_ok());
        assert!(parse_args(["--help"]).is_ok());
    }

    #[test]
    fn value_flags_still_accept_the_inline_form() {
        let args = parse_args([
            "--kind=sections",
            "--page-size=50",
            "--max-items=25",
            "--delay-ms=0",
        ])
        .expect("value flags parse");
        assert_eq!(args.kind, ArtifactKindFilter::Sections);
        assert_eq!(args.page_size, 50);
        assert_eq!(args.max_items, 25);
        assert_eq!(args.delay_ms, 0);
    }

    #[test]
    fn direction_and_mode_are_reported_explicitly() {
        let forward = parse_args(["--apply", "--acknowledge-reader-upgrade"]).expect("forward");
        assert_eq!(forward.direction(), ArtifactDirection::Forward);
        let report = ArtifactBackfillReport::new(&forward, true);
        assert_eq!(report.direction, "forward");
        assert_eq!(report.mode, "apply");
        assert_eq!(report.acknowledgement, Some(APPLY_ACKNOWLEDGEMENT));

        let restore = parse_args(["--restore-inline", "--apply", "--acknowledge-inline-growth"])
            .expect("restore");
        assert_eq!(restore.direction(), ArtifactDirection::RestoreInline);
        let report = ArtifactBackfillReport::new(&restore, true);
        assert_eq!(report.direction, "restore_inline");
        assert_eq!(report.acknowledgement, Some(RESTORE_ACKNOWLEDGEMENT));
    }

    #[test]
    fn base64_growth_is_computed_before_allocation() {
        assert_eq!(base64_encoded_len(0), Some(0));
        assert_eq!(base64_encoded_len(1), Some(4));
        assert_eq!(base64_encoded_len(3), Some(4));
        assert_eq!(base64_encoded_len(4), Some(8));
        assert_eq!(base64_encoded_len(16), Some(24));
        assert_eq!(base64_encoded_len(-1), None);
    }

    fn restore_row(kind: TaskItemArtifactKind) -> ArtifactRestoreRow {
        ArtifactRestoreRow {
            item_id: Uuid::nil(),
            task_id: Uuid::nil(),
            kind,
            encoding: kind.encoding().to_string(),
            storage_object_id: Uuid::nil(),
            object_group_id: 1,
            object_sha256: "a".repeat(64),
            object_size_bytes: 3,
            object_storage_backend: "local".to_string(),
            item_status: "succeeded".to_string(),
            task_status: "succeeded".to_string(),
            task_deleted: false,
            lease_active: false,
            task_group_id: Some(1),
            inline_type: None,
            download_metadata_type: Some("object".to_string()),
            declared_sha256: Some("a".repeat(64)),
        }
    }

    #[test]
    fn restore_classification_reports_explicit_skip_classes() {
        let mut row = restore_row(TaskItemArtifactKind::Download);
        assert_eq!(classify_restore_row(&row, 1024, "local"), None);
        row.inline_type = Some("string".to_string());
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("inline_present")
        );
        row.inline_type = None;
        row.lease_active = true;
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("active_item")
        );
        row.lease_active = false;
        row.task_group_id = None;
        assert_eq!(classify_restore_row(&row, 1024, "local"), Some("no_group"));
        row.task_group_id = Some(2);
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("group_mismatch")
        );
        row.task_group_id = Some(1);
        row.object_storage_backend = "s3".to_string();
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("inactive_backend")
        );
        row.object_storage_backend = "local".to_string();
        row.download_metadata_type = None;
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("missing_download_metadata")
        );
        row.download_metadata_type = Some("object".to_string());
        row.declared_sha256 = None;
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("missing_declared_sha")
        );
        row.declared_sha256 = Some("zz".to_string());
        assert_eq!(
            classify_restore_row(&row, 1024, "local"),
            Some("invalid_declared_sha")
        );
        row.declared_sha256 = Some("a".repeat(64));
        row.object_size_bytes = 1000;
        assert_eq!(classify_restore_row(&row, 1024, "local"), Some("oversize"));
        assert_eq!(classify_restore_row(&row, 4096, "local"), None);
    }

    #[test]
    fn restore_sections_classification_ignores_download_metadata() {
        let mut row = restore_row(TaskItemArtifactKind::Sections);
        row.download_metadata_type = None;
        row.declared_sha256 = None;
        assert_eq!(classify_restore_row(&row, 1024, "local"), None);
    }

    #[test]
    fn restore_query_requires_the_relation_and_reports_the_snapshot() {
        let restore = include_str!("../../sql/db/tasks/artifact_backfill/restore_candidates.sql");
        assert!(restore.contains("task_item_artifacts"));
        let remaining =
            include_str!("../../sql/db/tasks/artifact_backfill/remaining_artifact_references.sql");
        assert!(remaining.contains("task_item_artifacts"));
        let request = RestoreTaskItemArtifactInlineRequest {
            item_id: Uuid::nil(),
            kind: TaskItemArtifactKind::Download,
            storage_object_id: Uuid::nil(),
            sha256: "",
            size_bytes: 0,
            storage_backend: "local",
            encoding: "raw-v1",
            download_base64: None,
            sections_json: None,
        };
        assert_eq!(request.encoding, "raw-v1");
    }

    #[test]
    fn restore_report_redacts_content_and_digest_values() {
        let restore = parse_args(["--restore-inline", "--apply", "--acknowledge-inline-growth"])
            .expect("restore");
        let mut report = ArtifactBackfillReport::new(&restore, true);
        report.restored(TaskItemArtifactKind::Download, 4);
        report.remaining_artifact_references = Some(
            RemainingArtifactReferences {
                total: 1,
                download: 1,
                sections: 0,
                active: 0,
                terminal: 1,
                soft_deleted: 0,
            }
            .into(),
        );
        let json = serde_json::to_string(&report).expect("report serializes");
        assert!(json.contains("restore_inline"));
        assert!(json.contains("point_in_time"));
        assert!(!json.contains("download_artifact"));
        assert!(!json.contains("section_payload"));
        assert!(!json.contains("sha256"));
        assert!(!json.contains("base64"));
    }

    #[tokio::test]
    async fn shared_entry_help_finishes_without_config_or_database() {
        run_cli(["--help"]).await.expect("help succeeds");
    }

    #[tokio::test]
    async fn shared_entry_rejects_invalid_flags_before_any_lookup() {
        let unsupported = run_cli(["--kind", "bogus"])
            .await
            .expect_err("an unknown kind must fail");
        assert!(unsupported.to_string().contains("--kind must be"));
        let missing_ack = run_cli(["--restore-inline", "--apply"])
            .await
            .expect_err("restore apply needs its acknowledgement");
        assert!(
            missing_ack
                .to_string()
                .contains("--acknowledge-inline-growth")
        );
    }

    #[test]
    fn help_text_documents_both_directions_without_a_rollback_promise() {
        let help = help_text();
        assert!(help.contains("--restore-inline"));
        assert!(help.contains("--acknowledge-inline-growth"));
        assert!(help.contains("does not make"));
        assert!(!help.contains("safe to roll back"));
    }

    #[test]
    fn report_never_contains_payload_content_or_digest_values() {
        let mut report = ArtifactBackfillReport::new(&ArtifactBackfillArgs::default(), false);
        report.skip("invalid_sections");
        let json = serde_json::to_string(&report).expect("report serializes");
        assert!(json.contains("invalid_sections"));
        assert!(!json.contains("base64"));
        assert!(!json.contains("sha256"));
        assert!(!json.contains("source_url"));
    }
}
