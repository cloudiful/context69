import { describe, expect, it } from "vitest";

import {
  QueuePartialFailureError,
  QueueTaskSkippedError,
  availableQueueBulkActions,
  captureQueueBulkTargets,
  isQueueBulkActionApplicable,
  isSourceRerunnableTask,
  runQueueBatchRerun,
  runQueueBatchRetry,
  runQueueBulkTasks,
  summarizeQueueBulkOutcomes,
  type QueueBulkTarget,
} from "./queue-helpers";

function target(taskId: string, status: QueueBulkTarget["status"], action: QueueBulkTarget["action"]) {
  return { taskId, status, action } as QueueBulkTarget;
}

describe("availableQueueBulkActions", () => {
  it("offers an action only when a selected task can take it", () => {
    expect(availableQueueBulkActions([], "processing")).toEqual([]);
    expect(availableQueueBulkActions(["failed"], "processing")).toEqual(["retry", "trash"]);
    expect(availableQueueBulkActions(["cancelled"], "processing")).toEqual(["retry", "trash"]);
    expect(availableQueueBulkActions(["failed", "cancelled"], "processing"))
      .toEqual(["retry", "trash"]);
    expect(availableQueueBulkActions(["running"], "processing")).toEqual(["cancel", "trash"]);
    expect(availableQueueBulkActions(["succeeded"], "completed")).toEqual(["rerun", "trash"]);
  });

  it("applies each action only to the statuses it can act on", () => {
    expect(isQueueBulkActionApplicable("retry", "failed")).toBe(true);
    expect(isQueueBulkActionApplicable("retry", "cancelled")).toBe(true);
    expect(isQueueBulkActionApplicable("retry", "running")).toBe(false);
    expect(isQueueBulkActionApplicable("retry", "succeeded")).toBe(false);
    expect(isQueueBulkActionApplicable("rerun", "succeeded")).toBe(true);
    expect(isQueueBulkActionApplicable("rerun", "failed")).toBe(false);
    expect(isQueueBulkActionApplicable("cancel", "running")).toBe(true);
    expect(isQueueBulkActionApplicable("cancel", "succeeded")).toBe(false);
    expect(isQueueBulkActionApplicable("trash", "running")).toBe(true);
    expect(isQueueBulkActionApplicable("trash", "failed")).toBe(true);
    expect(isQueueBulkActionApplicable("restore", "failed")).toBe(true);
    expect(isQueueBulkActionApplicable("delete", "succeeded")).toBe(true);
  });

  it("keeps the trash view to restore and permanent deletion", () => {
    expect(availableQueueBulkActions(["failed"], "trash")).toEqual(["restore", "delete"]);
    expect(availableQueueBulkActions([], "trash")).toEqual([]);
  });
});

describe("captureQueueBulkTargets", () => {
  it("captures each selected id once with the status it had when selected", () => {
    const targets = captureQueueBulkTargets(
      ["a", "b", "a", "missing"],
      { a: "failed", b: "running" },
      "retry",
    );
    expect(targets).toEqual([
      { taskId: "a", status: "failed", action: "retry" },
      { taskId: "b", status: "running", action: "retry" },
    ]);
  });

  it("does not capture an id whose status is unknown", () => {
    expect(captureQueueBulkTargets(["ghost"], {}, "cancel")).toEqual([]);
  });
});

describe("runQueueBulkTasks", () => {
  it("never runs more than four tasks at a time and keeps the target order", async () => {
    const targets = Array.from({ length: 10 }, (_, index) => target(`task-${index}`, "failed", "retry"));
    let active = 0;
    let peak = 0;
    const progress: number[] = [];

    const outcomes = await runQueueBulkTasks(
      targets,
      async (item) => {
        active += 1;
        peak = Math.max(peak, active);
        await Promise.resolve();
        active -= 1;
        if (item.taskId === "task-5") {
          throw new QueueTaskSkippedError("not eligible");
        }
        return undefined;
      },
      (completed) => progress.push(completed),
    );

    expect(peak).toBeLessThanOrEqual(4);
    expect(outcomes.map((outcome) => outcome.taskId)).toEqual(targets.map((item) => item.taskId));
    expect(outcomes[5]?.status).toBe("skipped");
    expect(progress.at(-1)).toBe(10);
  });

  it("classifies an eligibility refusal as skipped and any other failure as failed", async () => {
    const outcomes = await runQueueBulkTasks(
      [
        target("conflict", "running", "cancel"),
        target("missing", "running", "cancel"),
        target("server", "running", "cancel"),
      ],
      async (item) => {
        if (item.taskId === "conflict") {
          throw Object.assign(new Error("already terminal"), { status: 409 });
        }
        if (item.taskId === "missing") {
          throw Object.assign(new Error("not found"), { status: 404 });
        }
        throw Object.assign(new Error("boom"), { status: 500 });
      },
    );

    expect(outcomes.map((outcome) => outcome.status)).toEqual(["skipped", "skipped", "failed"]);
  });

  it("keeps a partial failure distinguishable from a plain failure", async () => {
    const outcomes = await runQueueBulkTasks(
      [target("partial", "running", "trash")],
      async () => {
        throw new QueuePartialFailureError("cancelled but not trashed");
      },
    );
    expect(outcomes[0]?.status).toBe("failed");
    expect(outcomes[0]?.partial).toBe(true);
  });
});

describe("runQueueBatchRetry", () => {
  it("splits the captured ids into bounded chunks and maps ordered outcomes", async () => {
    const targets = Array.from({ length: 250 }, (_, index) => target(`task-${index}`, "failed", "retry"));
    const batches: string[][] = [];
    let active = 0;
    let peak = 0;
    const progress: number[] = [];

    const outcomes = await runQueueBatchRetry(
      targets,
      async (taskIds) => {
        active += 1;
        peak = Math.max(peak, active);
        await Promise.resolve();
        batches.push([...taskIds]);
        active -= 1;
        return {
          results: taskIds.map((taskId) => taskId === "task-1"
            ? { task_id: taskId, outcome: "skipped", reason: "nothing_to_retry" }
            : taskId === "task-2"
              ? { task_id: taskId, outcome: "failed", reason: "internal_error" }
              : { task_id: taskId, outcome: "accepted", retried_task_id: taskId }),
        };
      },
      (completed) => progress.push(completed),
    );

    expect(batches.map((batch) => batch.length)).toEqual([100, 100, 50]);
    expect(peak).toBeLessThanOrEqual(4);
    expect(outcomes.map((outcome) => outcome.taskId)).toEqual(targets.map((item) => item.taskId));
    expect(outcomes.map((outcome) => outcome.status).slice(0, 3))
      .toEqual(["succeeded", "skipped", "failed"]);
    expect(outcomes[1]?.reason).toBe("nothing_to_retry");
    expect(progress.at(-1)).toBe(250);
  });

  it("marks only the affected chunk when one batch request fails", async () => {
    const targets = Array.from({ length: 150 }, (_, index) => target(`task-${index}`, "cancelled", "retry"));
    const outcomes = await runQueueBatchRetry(targets, async (taskIds) => {
      if (taskIds[0] === "task-0") {
        throw new Error("transport down");
      }
      return {
        results: taskIds.map((taskId) => ({ task_id: taskId, outcome: "accepted", retried_task_id: taskId })),
      };
    });

    expect(outcomes.slice(0, 100).every((outcome) => outcome.status === "failed")).toBe(true);
    expect(outcomes.slice(100).every((outcome) => outcome.status === "succeeded")).toBe(true);
  });

  it("never reports a task the response omitted as recovered", async () => {
    const outcomes = await runQueueBatchRetry(
      [target("kept", "failed", "retry"), target("omitted", "failed", "retry")],
      async () => ({ results: [{ task_id: "kept", outcome: "accepted", retried_task_id: "kept" }] }),
    );
    expect(outcomes[0]?.status).toBe("succeeded");
    expect(outcomes[1]?.status).toBe("failed");
  });
});

describe("runQueueBatchRerun", () => {
  it("maps the ordered per-id outcomes of the source rerun", async () => {
    const targets = [
      target("succeeded-1", "succeeded", "rerun"),
      target("busy", "succeeded", "rerun"),
      target("missing", "succeeded", "rerun"),
    ];

    const outcomes = await runQueueBatchRerun(targets, async (taskIds) => ({
      results: taskIds.map((taskId) => taskId === "succeeded-1"
        ? { task_id: taskId, outcome: "accepted" as const, rerun_task_id: "child-1" }
        : taskId === "busy"
          ? { task_id: taskId, outcome: "skipped" as const, reason: "active_source_busy" as const }
          : { task_id: taskId, outcome: "skipped" as const, reason: "not_found" as const }),
    }));

    expect(outcomes.map((outcome) => outcome.taskId)).toEqual([
      "succeeded-1",
      "busy",
      "missing",
    ]);
    expect(outcomes.map((outcome) => outcome.status)).toEqual([
      "succeeded",
      "skipped",
      "skipped",
    ]);
    expect(outcomes[1]?.reason).toBe("active_source_busy");
  });

  it("never reports a task the response omitted as rerun", async () => {
    const outcomes = await runQueueBatchRerun(
      [target("kept", "succeeded", "rerun"), target("omitted", "succeeded", "rerun")],
      async () => ({ results: [{ task_id: "kept", outcome: "accepted" as const }] }),
    );
    expect(outcomes[0]?.status).toBe("succeeded");
    expect(outcomes[1]?.status).toBe("failed");
  });
});

describe("isSourceRerunnableTask", () => {
  it("offers the rerun only for a succeeded task that owns a source", () => {
    expect(isSourceRerunnableTask({ status: "succeeded", kind: "file_batch" } as never)).toBe(true);
    expect(isSourceRerunnableTask({ status: "succeeded", kind: "url_batch" } as never)).toBe(true);
    expect(isSourceRerunnableTask({ status: "succeeded", kind: "text_batch" } as never)).toBe(true);
    expect(isSourceRerunnableTask({ status: "succeeded", kind: "translation" } as never)).toBe(false);
    expect(isSourceRerunnableTask({ status: "succeeded", kind: "source_sync" } as never)).toBe(false);
    expect(isSourceRerunnableTask({ status: "failed", kind: "file_batch" } as never)).toBe(false);
  });
});

describe("summarizeQueueBulkOutcomes", () => {
  it("counts successes, skips, failures and partials separately", () => {
    const summary = summarizeQueueBulkOutcomes([
      { taskId: "a", status: "succeeded", partial: false },
      { taskId: "b", status: "skipped", partial: false },
      { taskId: "c", status: "failed", partial: true },
      { taskId: "d", status: "failed", partial: false },
    ]);
    expect(summary).toEqual({ succeeded: 1, skipped: 1, failed: 2, partial: 1 });
  });
});
