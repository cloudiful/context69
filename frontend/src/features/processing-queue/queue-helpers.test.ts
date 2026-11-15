import { describe, expect, it } from "vitest";

import type { TaskResponse } from "@/services/api";

import {
  QUEUE_STAGE_FILTER_VALUES,
  failureStageLabel,
  taskEffectiveStage,
  taskElapsedDuration,
  taskElapsedTicks,
  taskStageLabel,
} from "./queue-helpers";

function task(overrides: Partial<TaskResponse>): TaskResponse {
  return {
    stage: "processing",
    focus_progress: null,
    ...overrides,
  } as unknown as TaskResponse;
}

describe("queue effective stage", () => {
  it("uses the API effective stage and ignores rich focus progress", () => {
    expect(taskEffectiveStage(task({}))).toBe("processing");
    expect(taskEffectiveStage(task({ stage: null }))).toBeNull();
    expect(taskEffectiveStage(task({ stage: "indexing" }))).toBe("indexing");
    // The server already resolved the effective stage: malformed or absent rich
    // progress must never override it.
    expect(taskEffectiveStage(task({
      stage: "parsing",
      focus_progress: null,
    }))).toBe("parsing");
    expect(taskEffectiveStage(task({
      stage: "processing",
      focus_progress: { stage: "indexing", attempt: 2 },
    }))).toBe("processing");
  });

  it("labels the same stage it selects and offers both vocabularies to the filter", () => {
    const t = (key: string) => key;

    expect(taskStageLabel(t, task({ stage: "chunking" }))).toBe(
      "processingQueue.progressStageLabels.chunking",
    );
    expect(taskStageLabel(t, task({ stage: "finalize" }))).toBe("processingQueue.stages.finalize");

    for (const stage of ["parsing", "docling", "chunking", "finalizing", "processing"]) {
      expect(QUEUE_STAGE_FILTER_VALUES).toContain(stage);
    }
    expect(new Set(QUEUE_STAGE_FILTER_VALUES).size).toBe(QUEUE_STAGE_FILTER_VALUES.length);
  });

  it("keeps every nonempty failure stage visible, localized when known", () => {
    const t = (key: string) => key;

    expect(failureStageLabel(t, "parsing")).toBe("processingQueue.progressStageLabels.parsing");
    expect(failureStageLabel(t, "processing")).toBe("processingQueue.stages.processing");
    expect(failureStageLabel(t, "vendor-specific-stage")).toBe("vendor-specific-stage");
    expect(failureStageLabel(t, null)).toBeNull();
    expect(failureStageLabel(t, "")).toBeNull();
  });
});

describe("queue task elapsed", () => {
  const start = "2026-01-01T00:00:00Z";
  const startMs = Date.parse(start);

  it("ticks for every started nonterminal task, including waiting and paused", () => {
    for (const status of ["queued", "running", "waiting"] as const) {
      const entry = task({ status, started_at: start });
      expect(taskElapsedTicks(entry)).toBe(true);
      expect(taskElapsedDuration(entry, startMs + 91_000)).toBe("1m 31s");
    }
    // A queued task that never started has no elapsed time to tick.
    expect(taskElapsedTicks(task({ status: "queued", started_at: null }))).toBe(false);
  });

  it("uses the finished time for terminal tasks and never falls through to now", () => {
    const finished = task({
      status: "succeeded",
      started_at: start,
      finished_at: "2026-01-01T00:02:00Z",
    });
    expect(taskElapsedTicks(finished)).toBe(false);
    expect(taskElapsedDuration(finished, startMs + 3_600_000)).toBe("2m 0s");

    const missingFinish = task({ status: "failed", started_at: start, finished_at: null });
    expect(taskElapsedTicks(missingFinish)).toBe(false);
    expect(taskElapsedDuration(missingFinish, startMs + 3_600_000)).toBe("--");
  });
});
