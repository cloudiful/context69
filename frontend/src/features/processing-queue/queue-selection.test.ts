import { describe, expect, it } from "vitest";

import {
  QueuePartialFailureError,
  QueueTaskSkippedError,
  availableQueueBulkActions,
  captureQueueBulkTargets,
  isQueueBulkActionApplicable,
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
    expect(availableQueueBulkActions(["cancelled"], "processing")).toEqual(["resume", "trash"]);
    expect(availableQueueBulkActions(["running"], "processing")).toEqual(["cancel", "trash"]);
    expect(availableQueueBulkActions(["succeeded"], "completed")).toEqual(["trash"]);
  });

  it("applies each action only to the statuses it can act on", () => {
    expect(isQueueBulkActionApplicable("retry", "failed")).toBe(true);
    expect(isQueueBulkActionApplicable("retry", "cancelled")).toBe(false);
    expect(isQueueBulkActionApplicable("resume", "cancelled")).toBe(true);
    expect(isQueueBulkActionApplicable("resume", "failed")).toBe(false);
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
