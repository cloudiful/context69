//! The startup latch and the handover tool of the task execution engine
//! (issue 1020 P4A).
//!
//! Two things live here:
//!
//! - [`ExecutionLatch`], the one readiness decision a [`TaskService`] takes at
//!   startup. It is evaluated once, shared by every loop of both the regular and
//!   the upgrade-recovery profile, and never rechecked, so a deployment that
//!   starts with abandoned legacy work keeps its task engine off until it is
//!   handed over and restarted.
//! - the domain-owned command surface of the handover binary: strict argument
//!   parsing, the mandatory database target check, and the bounded execution of
//!   `inventory`, `adopt --item`, the bounded `adopt --all`, and
//!   `finalize-interrupted`.
//!
//! [`TaskService`]: super::TaskService

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::OnceCell;
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::Database;

/// How the operator resolves a blocked startup caused by legacy items.
pub const LEGACY_ITEMS_REMEDIATION_HINT: &str = "stop every task worker, hand the listed items over with task_execution_cutover (adopt --all, adopt --item or finalize-interrupted --item with --workers-stopped), then restart the process";

/// How the operator resolves a blocked startup caused by the library schema.
pub const LIBRARY_REMEDIATION_HINT: &str = "fix or remove a foreign library schema, or start the service once or run db_init to prepare a missing one, then restart the process";

/// The largest inventory page the tool asks for.
pub const MAX_INVENTORY_PAGE: i64 = 100;

/// The largest number of items one handover command accepts.
///
/// The bound is part of the tool contract: a batch is one transaction under the
/// admission lock, and the operator names every item explicitly.
pub const MAX_HANDOVER_ITEMS: usize = 100;

/// The latched readiness of the task execution engine.
///
/// The vocabulary belongs to the service, not to the database layer: the
/// decision is a fact about this process starting the engine, and the stable
/// blocker code is what the log line and the operator wording use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionReadiness {
    /// No legacy work and a genuine library schema: the engine may start.
    Ready,
    /// The engine must not start until the blocker is remediated.
    Blocked {
        /// The stable code of the blocker.
        code: &'static str,
        /// The number of unlinked non-terminal items, when that is the blocker.
        unlinked_items: i64,
    },
}

impl ExecutionReadiness {
    /// Whether the engine may start.
    #[must_use]
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The stable code of the blocker, when there is one.
    #[must_use]
    pub fn blocker_code(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Blocked { code, .. } => Some(code),
        }
    }
}

/// The one readiness decision of one task service.
///
/// The decision is taken once, by whichever loop reaches it first, and is never
/// re-evaluated: a handover performed while the process runs does not open the
/// engine under it, so remediation always ends with a restart.
pub struct ExecutionLatch {
    decision: OnceCell<ExecutionReadiness>,
}

impl ExecutionLatch {
    /// A latch that has not decided yet.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            decision: OnceCell::new(),
        })
    }

    /// The readiness of the engine, decided once per process.
    ///
    /// A failing check is a blocker, never an allowance: the engine fails
    /// closed.
    pub async fn readiness(&self, db: &Database) -> ExecutionReadiness {
        *self
            .decision
            .get_or_init(|| async {
                let decision = match db.cutover_readiness().await {
                    Ok(decision) if decision.is_ready() => ExecutionReadiness::Ready,
                    Ok(decision) => ExecutionReadiness::Blocked {
                        code: decision.blocker_code().unwrap_or("library"),
                        unlinked_items: decision.unlinked_item_count(),
                    },
                    Err(error) => {
                        let message = error.to_string();
                        warn!(
                            blocker = "library",
                            error = %Sanitized(&message),
                            "task execution readiness could not be checked",
                        );
                        ExecutionReadiness::Blocked {
                            code: "library",
                            unlinked_items: 0,
                        }
                    }
                };
                match decision {
                    ExecutionReadiness::Ready => {
                        info!(
                            event = "ready",
                            "task execution engine authorized to start",
                        );
                    }
                    ExecutionReadiness::Blocked {
                        code,
                        unlinked_items,
                    } => {
                        let hint = match code {
                            "legacy-items" => LEGACY_ITEMS_REMEDIATION_HINT,
                            _ => LIBRARY_REMEDIATION_HINT,
                        };
                        warn!(
                            event = "blocked",
                            blocker = code,
                            unlinked_items,
                            hint,
                            "task execution engine refused to start: no dispatcher, no maintenance, no docling recovery",
                        );
                    }
                }
                decision
            })
            .await
    }

    /// The decided readiness, once the decision was taken.
    #[must_use]
    pub fn decided(&self) -> Option<ExecutionReadiness> {
        self.decision.get().copied()
    }
}

/// The command one cutover invocation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutoverCommand {
    /// The read-only inventory of the abandoned items.
    Inventory {
        /// The page size, at most [`MAX_INVENTORY_PAGE`].
        limit: i64,
        /// The page offset.
        offset: i64,
    },
    /// Links a fresh execution segment to the named unlinked items.
    Adopt {
        /// The explicitly named items.
        item_ids: Vec<Uuid>,
    },
    /// Links fresh execution segments to one bounded batch of eligible items.
    ///
    /// The selection is applied in SQL to unlinked `queued`/`waiting` items
    /// with an active parent before the bound; running, terminal and
    /// retired-parent items are never selected, and the operator repeats the
    /// command only while the reported adoptable remainder is non-zero.
    AdoptAll,
    /// Closes the named unlinked running items whose lease is gone.
    FinalizeInterrupted {
        /// The explicitly named items.
        item_ids: Vec<Uuid>,
    },
}

/// One parsed cutover invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutoverCli {
    /// The command to run.
    pub command: CutoverCommand,
    /// The database name the operator claims to target.
    pub expect_database: String,
    /// Whether the operator confirmed that every task worker is stopped.
    pub workers_stopped: bool,
}

/// Why one invocation could not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverCliErrorKind {
    /// The arguments are invalid.
    Usage,
    /// The invoked database is not the expected one.
    Target,
    /// A safety prerequisite is not met.
    Safety,
}

/// The refusal of one invocation, with the exit code the tool reports.
#[derive(Debug, Clone)]
pub struct CutoverCliError {
    /// The class of the refusal.
    pub kind: CutoverCliErrorKind,
    /// The concise, credential-free reason.
    pub message: String,
}

impl CutoverCliError {
    /// The process exit code of this refusal.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self.kind {
            CutoverCliErrorKind::Usage | CutoverCliErrorKind::Target => 2,
            CutoverCliErrorKind::Safety => 3,
        }
    }

    fn new(kind: CutoverCliErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Redacts every credential a message may carry.
///
/// Any `scheme://user:secret@host` authority is reduced to `scheme://host`, so
/// a driver error or a connection string can be pasted anywhere without leaking
/// a credential. The rest of the message is kept verbatim.
pub struct Sanitized<'a>(pub &'a str);

impl std::fmt::Display for Sanitized<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut rest = self.0;
        while let Some(scheme_end) = rest.find("://") {
            write!(formatter, "{}://", &rest[..scheme_end])?;
            let after = &rest[scheme_end + 3..];
            let end = after
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, '"' | '\'' | ')' | ',')
                })
                .unwrap_or(after.len());
            let authority = &after[..end];
            match authority.rfind('@') {
                Some(at) => write!(formatter, "{}", &authority[at + 1..])?,
                None => write!(formatter, "{authority}")?,
            }
            rest = &after[end..];
        }
        write!(formatter, "{rest}")
    }
}

/// Parses one cutover invocation.
///
/// Every flag is parsed here, before any connection or credential is touched:
/// an unknown flag, a missing `--expect-database`, a missing `--workers-stopped`,
/// an out-of-range page or an oversized item set is refused without a database
/// round trip.
pub fn parse_cutover_args(args: &[String]) -> Result<CutoverCli, CutoverCliError> {
    let mut args = args.iter();
    let mut expect_database: Option<String> = None;
    let mut workers_stopped = false;
    let mut limit: Option<i64> = None;
    let mut offset: Option<i64> = None;
    let mut item_ids: Vec<Uuid> = Vec::new();
    let mut all = false;
    let mut command: Option<&str> = None;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "inventory" | "adopt" | "finalize-interrupted" => {
                if command.is_some() {
                    return Err(usage("exactly one command may be given"));
                }
                command = Some(argument);
            }
            "--expect-database" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage("--expect-database needs a value"))?;
                expect_database = Some(value.clone());
            }
            "--workers-stopped" => workers_stopped = true,
            "--all" => {
                if all {
                    return Err(usage("--all may be given at most once"));
                }
                all = true;
            }
            "--limit" => {
                let value = args.next().ok_or_else(|| usage("--limit needs a value"))?;
                limit = Some(
                    value
                        .parse::<i64>()
                        .map_err(|_| usage("--limit must be an integer"))?,
                );
            }
            "--offset" => {
                let value = args.next().ok_or_else(|| usage("--offset needs a value"))?;
                offset = Some(
                    value
                        .parse::<i64>()
                        .map_err(|_| usage("--offset must be an integer"))?,
                );
            }
            "--item" => {
                let value = args.next().ok_or_else(|| usage("--item needs a value"))?;
                item_ids.push(Uuid::parse_str(value).map_err(|_| usage("--item must be a UUID"))?);
            }
            other => return Err(usage(format!("unknown argument `{other}`"))),
        }
    }

    let command = command
        .ok_or_else(|| usage("one of inventory, adopt or finalize-interrupted is required"))?;
    let expect_database = expect_database.ok_or_else(|| usage("--expect-database is required"))?;
    if expect_database.trim().is_empty() {
        return Err(usage("--expect-database must not be empty"));
    }
    let command = match command {
        "inventory" => {
            if !item_ids.is_empty() || workers_stopped || all {
                return Err(usage(
                    "inventory takes neither --item, --all nor --workers-stopped",
                ));
            }
            let limit = limit.unwrap_or(MAX_INVENTORY_PAGE);
            let offset = offset.unwrap_or(0);
            if !(1..=MAX_INVENTORY_PAGE).contains(&limit) || offset < 0 {
                return Err(usage(format!(
                    "inventory page must be 1..{} with a non-negative offset",
                    MAX_INVENTORY_PAGE
                )));
            }
            CutoverCommand::Inventory { limit, offset }
        }
        "adopt" | "finalize-interrupted" => {
            if limit.is_some() || offset.is_some() {
                return Err(usage("--limit and --offset are inventory-only"));
            }
            if !workers_stopped {
                return Err(usage(
                    "adopt and finalize-interrupted require --workers-stopped",
                ));
            }
            if all && command == "finalize-interrupted" {
                return Err(usage("--all is adopt-only"));
            }
            if all && !item_ids.is_empty() {
                return Err(usage("--all and --item are mutually exclusive"));
            }
            if !all && item_ids.is_empty() {
                return Err(usage(
                    "at least one --item or --all is required; there is no implicit all",
                ));
            }
            if item_ids.len() > MAX_HANDOVER_ITEMS {
                return Err(usage(format!(
                    "at most {} items may be handed over in one call",
                    MAX_HANDOVER_ITEMS
                )));
            }
            let mut sorted = item_ids.clone();
            sorted.sort_unstable();
            sorted.dedup();
            if sorted.len() != item_ids.len() {
                return Err(usage("--item ids must be unique"));
            }
            if command == "adopt" {
                if all {
                    CutoverCommand::AdoptAll
                } else {
                    CutoverCommand::Adopt { item_ids }
                }
            } else {
                CutoverCommand::FinalizeInterrupted { item_ids }
            }
        }
        other => return Err(usage(format!("unknown command `{other}`"))),
    };

    Ok(CutoverCli {
        command,
        expect_database,
        workers_stopped,
    })
}

fn usage(message: impl Into<String>) -> CutoverCliError {
    CutoverCliError::new(CutoverCliErrorKind::Usage, message)
}

/// The database name one connection string configures.
///
/// The name is read from the URL itself, so an operator who points the tool at
/// the wrong database is refused before the tool runs anything against it. The
/// URL is never echoed, and a URL without a database name is a usage error.
pub fn configured_database_name(database_url: &str) -> Result<String, CutoverCliError> {
    if database_url.trim().is_empty() {
        return Err(usage("DATABASE_URL is not set"));
    }
    let after_scheme = database_url
        .split_once("://")
        .map_or(database_url, |(_, rest)| rest);
    let path = after_scheme
        .split_once('/')
        .map(|(_, rest)| rest)
        .ok_or_else(|| usage("DATABASE_URL carries no database name"))?;
    let name = path.split(['?', '#']).next().unwrap_or_default();
    if name.is_empty() {
        return Err(usage("DATABASE_URL carries no database name"));
    }
    Ok(name.to_string())
}

/// The result of one invocation, ready to be printed as JSON metadata.
#[derive(Debug, Clone)]
pub struct CutoverRunOutcome {
    /// The command that ran.
    pub command: &'static str,
    /// The number of rows the command reported or changed.
    pub affected: u64,
    /// The number of already-linked items a repeated adoption skipped.
    pub already_linked: u64,
    /// The JSON metadata of the invocation.
    pub metadata: serde_json::Value,
}

/// Runs one parsed invocation against an open database.
pub async fn run_cutover(
    db: &Database,
    cli: &CutoverCli,
) -> Result<CutoverRunOutcome, CutoverCliError> {
    let actual = db.current_database_name().await.map_err(|error| {
        safety(format!(
            "database identity check failed: {}",
            Sanitized(&error.to_string())
        ))
    })?;
    if actual != cli.expect_database {
        return Err(CutoverCliError::new(
            CutoverCliErrorKind::Target,
            format!(
                "target database `{actual}` is not the expected `{}`",
                cli.expect_database
            ),
        ));
    }
    if !db.cutover_library_ready().await {
        return Err(safety(
            "the library schema is missing, foreign or incompatible; start the service once or run db_init to prepare it",
        ));
    }

    match &cli.command {
        CutoverCommand::Inventory { limit, offset } => {
            let rows = db
                .cutover_inventory(*limit, *offset)
                .await
                .map_err(|error| {
                    safety(format!(
                        "inventory failed: {}",
                        Sanitized(&error.to_string())
                    ))
                })?;
            let items = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "item_id": row.item_id,
                        "task_id": row.task_id,
                        "status": row.status,
                        "kind": row.kind,
                        "ordinal": row.ordinal,
                        "parent_retired": row.parent_retired,
                        "execution_generation": row.execution_generation,
                        "linked": row.execution_task_id.is_some(),
                        "next_attempt_at": row.next_attempt_at,
                        "docling_waiting": row.docling_waiting,
                    })
                })
                .collect::<Vec<_>>();
            Ok(CutoverRunOutcome {
                command: "inventory",
                affected: items.len() as u64,
                already_linked: 0,
                metadata: serde_json::json!({
                    "command": "inventory",
                    "limit": limit,
                    "offset": offset,
                    "count": items.len(),
                    "items": items,
                }),
            })
        }
        CutoverCommand::Adopt { item_ids } => {
            require_quiescence(db).await?;
            let outcome = db.cutover_adopt(item_ids).await.map_err(|error| {
                safety(format!("adopt refused: {}", Sanitized(&error.to_string())))
            })?;
            Ok(CutoverRunOutcome {
                command: "adopt",
                affected: outcome.adopted,
                already_linked: outcome.already_linked,
                metadata: serde_json::json!({
                    "command": "adopt",
                    "all": false,
                    "adopted": outcome.adopted,
                    "already_linked": outcome.already_linked,
                }),
            })
        }
        CutoverCommand::AdoptAll => {
            require_quiescence(db).await?;
            let outcome = db.cutover_adopt_all().await.map_err(|error| {
                safety(format!(
                    "adopt --all refused: {}",
                    Sanitized(&error.to_string())
                ))
            })?;
            Ok(CutoverRunOutcome {
                command: "adopt",
                affected: outcome.adopted,
                already_linked: outcome.already_linked,
                metadata: serde_json::json!({
                    "command": "adopt",
                    "all": true,
                    "adopted": outcome.adopted,
                    "already_linked": outcome.already_linked,
                    "remaining_adoptable": outcome.remaining_adoptable,
                    "total_blockers": outcome.total_blockers,
                }),
            })
        }
        CutoverCommand::FinalizeInterrupted { item_ids } => {
            require_quiescence(db).await?;
            let outcome = db
                .cutover_finalize_interrupted(item_ids)
                .await
                .map_err(|error| {
                    safety(format!(
                        "finalize-interrupted refused: {}",
                        Sanitized(&error.to_string())
                    ))
                })?;
            Ok(CutoverRunOutcome {
                command: "finalize-interrupted",
                affected: outcome.finalized,
                already_linked: 0,
                metadata: serde_json::json!({
                    "command": "finalize-interrupted",
                    "finalized": outcome.finalized,
                }),
            })
        }
    }
}

/// Refuses a write while either authority still holds a live lease.
async fn require_quiescence(db: &Database) -> Result<(), CutoverCliError> {
    let leases = db.cutover_live_execution_leases().await.map_err(|error| {
        safety(format!(
            "quiescence check failed: {}",
            Sanitized(&error.to_string())
        ))
    })?;
    if leases.any() {
        return Err(safety(
            "a live execution lease is held; stop every task worker before handing over",
        ));
    }
    Ok(())
}

fn safety(message: impl Into<String>) -> CutoverCliError {
    CutoverCliError::new(CutoverCliErrorKind::Safety, message)
}
