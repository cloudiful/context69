import { describe, expect, it } from "vitest";

import {
  QueuePartialFailureError,
  QueueRiskPrescanError,
  QueueTaskSkippedError,
  availableQueueBulkActions,
  buildQueueBulkReport,
  captureQueueBulkTargets,
  isQueueBulkActionApplicable,
  isSourceRerunnableTask,
  queueBulkActionOptions,
  readQueueBulkRisk,
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
      { taskId: "a", status: "failed", action: "retry", doclingUncertain: false, subject: null },
      { taskId: "b", status: "running", action: "retry", doclingUncertain: false, subject: null },
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

  it("keeps the accepted target of each batch item", async () => {
    const retried = await runQueueBatchRetry(
      [target("failed-1", "failed", "retry")],
      async () => ({
        results: [{ task_id: "failed-1", outcome: "accepted", retried_task_id: "failed-1" }],
      }),
    );
    expect(retried[0]?.targetTaskId).toBe("failed-1");

    const rerun = await runQueueBatchRerun(
      [target("parent-1", "succeeded", "rerun")],
      async () => ({
        results: [{ task_id: "parent-1", outcome: "accepted", rerun_task_id: "child-1" }],
      }),
    );
    expect(rerun[0]?.targetTaskId).toBe("child-1");
    expect(rerun[0]?.taskId).toBe("parent-1");

    const withoutChild = await runQueueBatchRerun(
      [target("parent-2", "succeeded", "rerun")],
      async () => ({ results: [{ task_id: "parent-2", outcome: "accepted" }] }),
    );
    expect(withoutChild[0]?.targetTaskId).toBeNull();
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
      { taskId: "a", status: "succeeded", partial: false, targetTaskId: "a" },
      { taskId: "b", status: "skipped", partial: false, targetTaskId: null },
      { taskId: "c", status: "failed", partial: true, targetTaskId: null },
      { taskId: "d", status: "failed", partial: false, targetTaskId: null },
    ]);
    expect(summary).toEqual({ succeeded: 1, skipped: 1, failed: 2, partial: 1 });
  });
});

describe("queueBulkActionOptions", () => {
  const t = (key: string, params?: Record<string, unknown>) => `${key}|${JSON.stringify(params ?? {})}`;

  it("labels recovery by the eligible failed and cancelled counts", () => {
    const onlyFailed = queueBulkActionOptions(t, ["failed", "failed"], "processing");
    expect(onlyFailed.find((option) => option.action === "retry")).toEqual({
      action: "retry",
      label: 'processingQueue.bulkActionLabels.retryFailed|{"count":2}',
      eligible: 2,
    });

    const onlyCancelled = queueBulkActionOptions(t, ["cancelled"], "processing");
    expect(onlyCancelled.find((option) => option.action === "retry")?.label)
      .toBe('processingQueue.bulkActionLabels.resumeCancelled|{"count":1}');

    const mixed = queueBulkActionOptions(t, ["failed", "cancelled", "cancelled"], "processing");
    expect(mixed.find((option) => option.action === "retry")).toEqual({
      action: "retry",
      label: 'processingQueue.bulkActionLabels.retryMixed|{"failed":1,"cancelled":2}',
      eligible: 3,
    });
  });

  it("never counts an unsupported selection in another action's label", () => {
    const options = queueBulkActionOptions(t, ["failed", "running", "running"], "processing");
    expect(options.find((option) => option.action === "retry"))
      .toEqual({
        action: "retry",
        label: 'processingQueue.bulkActionLabels.retryFailed|{"count":1}',
        eligible: 1,
      });
    expect(options.find((option) => option.action === "cancel")?.eligible).toBe(2);
    expect(options.find((option) => option.action === "rerun")).toBeUndefined();
  });
});

describe("buildQueueBulkReport", () => {
  it("maps outcomes to the visible accepted/skipped/failed vocabulary", () => {
    const report = buildQueueBulkReport("rerun", [
      {
        taskId: "parent-1",
        status: "succeeded",
        partial: false,
        targetTaskId: "child-1",
        subject: "Annual Report",
      },
      {
        taskId: "parent-2",
        status: "skipped",
        partial: false,
        targetTaskId: null,
        subject: null,
        reason: "active_source_busy",
      },
      {
        taskId: "parent-3",
        status: "failed",
        partial: true,
        targetTaskId: null,
        subject: "Quarterly",
        reason: "internal_error",
      },
    ]);

    expect(report.action).toBe("rerun");
    expect(report.summary).toEqual({ succeeded: 1, skipped: 1, failed: 1, partial: 1 });
    expect(report.entries).toEqual([
      {
        taskId: "parent-1",
        subject: "Annual Report",
        status: "accepted",
        targetTaskId: "child-1",
        reason: null,
      },
      {
        taskId: "parent-2",
        subject: null,
        status: "skipped",
        targetTaskId: null,
        reason: "active_source_busy",
      },
      {
        taskId: "parent-3",
        subject: "Quarterly",
        status: "failed",
        targetTaskId: null,
        reason: "internal_error",
      },
    ]);
  });
});

describe("duplicate-risk eligibility and prescan", () => {
  it("offers recovery for a waiting task only when its risk flag is explicit", () => {
    expect(isQueueBulkActionApplicable("retry", "waiting")).toBe(false);
    expect(isQueueBulkActionApplicable("retry", "waiting", true)).toBe(true);
    expect(availableQueueBulkActions(["waiting"], "processing")).toEqual(["cancel", "trash"]);
    expect(availableQueueBulkActions(["waiting"], "processing", 1))
      .toEqual(["retry", "cancel", "trash"]);
  });

  it("reads every selected id through bounded chunks in the requested order", async () => {
    const ids = Array.from({ length: 150 }, (_, index) => `task-${index}`);
    const calls: string[][] = [];
    const flags = await readQueueBulkRisk(ids, async (chunk) => {
      calls.push([...chunk]);
      return {
        items: chunk.map((taskId) => ({
          task_id: taskId,
          outcome: "found",
          task: { task_id: taskId, docling_submission_uncertain: taskId === "task-100" },
        })),
      } as never;
    });

    expect(calls.map((chunk) => chunk.length)).toEqual([100, 50]);
    expect(calls[0]?.[0]).toBe("task-0");
    expect(calls[1]?.[0]).toBe("task-100");
    expect(flags.get("task-100")).toBe(true);
    expect(flags.get("task-0")).toBe(false);
    expect(flags.size).toBe(150);
  });

  it("fails closed when a chunk omits or cannot read one selected id", async () => {
    await expect(readQueueBulkRisk(["task-1"], async () => ({
      items: [{ task_id: "task-1", outcome: "not_found" }],
    }))).rejects.toBeInstanceOf(QueueRiskPrescanError);

    await expect(readQueueBulkRisk(["task-1"], async () => {
      throw new Error("chunk failed");
    })).rejects.toThrow("chunk failed");
  });
});
