//! Snapshot-then-deltas producer for one SSE connection, plus the per-task
//! `update` coalescer and terminal-state helper it drives.

use std::collections::{HashMap, HashSet};

use context69_contracts::{
    TaskListQuery, TaskListView, TaskResponse, TaskStatus, TaskStreamDone, TaskStreamEvent,
    TaskStreamSnapshot, TaskStreamUpdate,
};
use tokio::sync::mpsc;
use uuid::Uuid;

pub(super) fn is_terminal_task_status(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
    )
}

/// Issue 413 Phase 2: per-task SSE `update` coalesce window (3s, user-confirmed).
/// The `snapshot` frame stays immediate; `update` frames buffer the latest full
/// state per `task_id` and flush on this interval. Keep-alive is unchanged.
pub(super) const TASK_STREAM_UPDATE_COALESCE_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(3);

/// Per-task latest-wins buffer for SSE `update` frames. Bursty bus events for
/// the same `task_id` collapse to one frame per
/// [`TASK_STREAM_UPDATE_COALESCE_WINDOW`]; distinct tasks each keep their
/// latest state. `drain` sorts by `task_id` so flush order is deterministic.
#[derive(Debug, Default)]
pub(super) struct TaskUpdateCoalescer {
    pub(super) pending: HashMap<Uuid, TaskResponse>,
}

impl TaskUpdateCoalescer {
    pub(super) fn push(&mut self, task: TaskResponse) {
        self.pending.insert(task.task_id, task);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(super) fn drain(&mut self) -> Vec<TaskResponse> {
        let mut out: Vec<TaskResponse> = self.pending.drain().map(|(_, task)| task).collect();
        out.sort_by_key(|task| task.task_id);
        out
    }
}

/// Snapshot-then-deltas producer for one SSE connection. The snapshot is sent
/// immediately; deltas coalesce per task on [`TASK_STREAM_UPDATE_COALESCE_WINDOW`]
/// (latest-wins) before `update` frames flush. Snapshot errors and lag
/// notifications are delivered as in-band `error` frames; foreign or missing
/// tasks are silently skipped (own-tasks-only filter).
pub(super) async fn run_task_stream(
    tasks: crate::services::tasks::TaskService,
    user_id: i64,
    watched_order: Vec<Uuid>,
    tx: mpsc::Sender<TaskStreamEvent>,
    mut abort: context69_search::AbortSignal,
) -> anyhow::Result<()> {
    let watched_set: HashSet<Uuid> = watched_order.iter().copied().collect();
    let filtering = !watched_set.is_empty();

    let snapshot_tasks: Vec<TaskResponse> = if filtering {
        let mut out = Vec::with_capacity(watched_order.len());
        for task_id in &watched_order {
            let fetched = tokio::select! {
                biased;
                _ = abort.wait() => return Ok(()),
                result = tasks.get(*task_id, user_id) => result,
            };
            match fetched {
                Ok(task) => out.push(task),
                Err(error) => {
                    if crate::domain_errors::is_not_found_error(&error) {
                        continue;
                    }
                    if matches!(
                        crate::domain_errors::find_domain_error(&error),
                        Some(crate::domain_errors::DomainError::Forbidden(_))
                    ) {
                        continue;
                    }
                    let _ = tx
                        .send(TaskStreamEvent::Error {
                            message: error.to_string(),
                        })
                        .await;
                    return Ok(());
                }
            }
        }
        out
    } else {
        let query = TaskListQuery {
            page: 1,
            page_size: 100,
            query: None,
            kind: None,
            status: None,
            view: Some(TaskListView::Processing),
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
        };
        let listed = tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            result = tasks.list(user_id, &query) => result,
        };
        match listed {
            Ok(page) => page.items,
            Err(error) => {
                let _ = tx
                    .send(TaskStreamEvent::Error {
                        message: error.to_string(),
                    })
                    .await;
                return Ok(());
            }
        }
    };

    if !send_task_frame(
        &tx,
        TaskStreamEvent::Snapshot(TaskStreamSnapshot {
            tasks: snapshot_tasks.clone(),
        }),
    )
    .await
    {
        return Ok(());
    }

    let mut known: HashMap<Uuid, TaskResponse> = snapshot_tasks
        .iter()
        .map(|task| (task.task_id, task.clone()))
        .collect();
    if filtering
        && known.len() == watched_set.len()
        && !known.is_empty()
        && known
            .values()
            .all(|task| is_terminal_task_status(&task.status))
    {
        let done_tasks = watched_order
            .iter()
            .filter_map(|id| known.get(id).cloned())
            .collect();
        let _ = send_task_frame(
            &tx,
            TaskStreamEvent::Done(TaskStreamDone { tasks: done_tasks }),
        )
        .await;
        return Ok(());
    }

    let mut subscription = tasks.subscribe_task_events();
    // Issue 413 Phase 2: snapshot above is immediate; updates below coalesce
    // per task_id on a 3s window. Bus events only refresh `known` plus the
    // latest pending state; the tick drains one `update` per dirty task, so a
    // burst for one task costs one frame per window. `done` follows the flush
    // that makes every watched task terminal, keeping update-then-done order.
    let mut pending = TaskUpdateCoalescer::default();
    let flush_start = tokio::time::Instant::now() + TASK_STREAM_UPDATE_COALESCE_WINDOW;
    let mut flush_tick = tokio::time::interval_at(flush_start, TASK_STREAM_UPDATE_COALESCE_WINDOW);
    flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            _ = flush_tick.tick() => {
                if pending.is_empty() {
                    continue;
                }
                for full in pending.drain() {
                    if !send_task_frame(
                        &tx,
                        TaskStreamEvent::Update(Box::new(TaskStreamUpdate { task: full })),
                    )
                    .await
                    {
                        return Ok(());
                    }
                }
                if filtering
                    && known.len() == watched_set.len()
                    && known.values().all(|task| is_terminal_task_status(&task.status))
                {
                    let done_tasks = watched_order
                        .iter()
                        .filter_map(|id| known.get(id).cloned())
                        .collect();
                    let _ = send_task_frame(
                        &tx,
                        TaskStreamEvent::Done(TaskStreamDone { tasks: done_tasks }),
                    )
                    .await;
                    return Ok(());
                }
            }
            received = subscription.recv() => match received {
                Ok(bus_event) => {
                    if filtering && !watched_set.contains(&bus_event.task_id) {
                        continue;
                    }
                    let fetched = tokio::select! {
                        biased;
                        _ = abort.wait() => return Ok(()),
                        result = tasks.get(bus_event.task_id, user_id) => result,
                    };
                    match fetched {
                        Ok(full) => {
                            known.insert(full.task_id, full.clone());
                            pending.push(full);
                        }
                        Err(error) => {
                            if crate::domain_errors::is_not_found_error(&error) {
                                continue;
                            }
                            if matches!(
                                crate::domain_errors::find_domain_error(&error),
                                Some(crate::domain_errors::DomainError::Forbidden(_))
                            ) {
                                continue;
                            }
                            let _ = tx
                                .send(TaskStreamEvent::Error {
                                    message: error.to_string(),
                                })
                                .await;
                            return Ok(());
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = tx
                        .send(TaskStreamEvent::Error {
                            message: "task events lagged; resync via GET /v1/tasks and reconnect"
                                .to_string(),
                        })
                        .await;
                    return Ok(());
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
            },
        }
    }
}

async fn send_task_frame(tx: &mpsc::Sender<TaskStreamEvent>, event: TaskStreamEvent) -> bool {
    tx.send(event).await.is_ok()
}
