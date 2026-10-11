import { describe, expect, it } from "vitest";

import {
  PIPELINE_PROGRESS_STAGES,
  itemProgressView,
  pipelineProgressStageLabel,
  progressCountText,
  progressPercent,
  type TaskItemProgress,
} from "./task-item-progress";

type Translate = (key: string, params?: Record<string, unknown>) => string;

const LABELS: Record<string, string> = {
  "processingQueue.unknownStage": "Unknown",
  "processingQueue.progressStageLabels.storage": "Storing",
  "processingQueue.progressStageLabels.parsing": "Parsing",
  "processingQueue.progressStageLabels.docling": "Converting remotely",
  "processingQueue.progressStageLabels.chunking": "Chunking",
  "processingQueue.progressStageLabels.indexing": "Indexing",
  "processingQueue.progressStageLabels.finalizing": "Finalizing",
};

// The real catalogues are not needed here: these tests are about the rendering
// rules, and a key-based stub makes a missing locale key fail loudly instead
// of silently rendering as the raw key.
function translate(this: unknown, key: string, params?: Record<string, unknown>): string {
  const unit = key.split(".").pop();
  if (unit === "chunksCount") return `${params?.completed ?? ""} / ${params?.total ?? ""} chunks`;
  if (unit === "batchesCount") return `${params?.completed ?? ""} / ${params?.total ?? ""} batches`;
  if (key in LABELS) return LABELS[key] as string;
  // Falling through to the key makes a missing locale entry fail loudly
  // instead of silently rendering as a readable-looking string.
  return key;
}
const tr = translate as Translate;

function progress(overrides: Partial<TaskItemProgress> = {}): TaskItemProgress {
  return { stage: "indexing", attempt: 1, ...overrides };
}

describe("pipelineProgressStageLabel", () => {
  it("labels every progress stage in the vocabulary", () => {
    for (const stage of PIPELINE_PROGRESS_STAGES) {
      expect(pipelineProgressStageLabel(tr, stage)).not.toBe("Unknown");
      expect(pipelineProgressStageLabel(tr, stage)).not.toContain("processingQueue.");
    }
  });

  // A progress stage this build does not know must not leak a raw identifier
  // or a raw i18n key into the UI.
  it("falls back to Unknown for an unknown, empty or missing stage", () => {
    expect(pipelineProgressStageLabel(tr, "teleporting")).toBe("Unknown");
    expect(pipelineProgressStageLabel(tr, "")).toBe("Unknown");
    expect(pipelineProgressStageLabel(tr, null)).toBe("Unknown");
  });
});

describe("progressPercent", () => {
  it("returns a percentage only for a known total greater than zero", () => {
    expect(progressPercent({ completed: 32, total: 512 })).toBe(6);
    expect(progressPercent({ completed: 1, total: 3 })).toBe(33);
  });

  // An unknown total has no denominator, and a zero total is already complete.
  // Neither may be turned into a number, because any number would be invented.
  it("returns null for an unknown or zero total", () => {
    expect(progressPercent(null)).toBeNull();
    expect(progressPercent({ completed: 0, total: 0 })).toBeNull();
  });
});

describe("itemProgressView", () => {
  it("keeps chunk and batch counters as separate units", () => {
    const view = itemProgressView(tr, progress({
      completed_chunks: 70,
      total_chunks: 512,
      completed_batches: 3,
      total_batches: 16,
    }))!;
    expect(view.chunks).toEqual({ completed: 70, total: 512 });
    expect(view.batches).toEqual({ completed: 3, total: 16 });
    expect(progressCountText(tr, view.chunks, "chunks")).toBe("70 / 512 chunks");
    expect(progressCountText(tr, view.batches, "batches")).toBe("3 / 16 batches");
  });

  it("omits a counter the server did not report instead of showing zero", () => {
    const view = itemProgressView(tr, progress({
      total_chunks: 512,
      total_batches: 16,
    }))!;
    // A resume publishes totals before it has re-counted its skipped chunks.
    expect(view.chunks).toBeNull();
    expect(view.batches).toBeNull();
    expect(view.isUnknown).toBe(true);
  });

  // A completed count larger than its total is not a believable observation;
  // rendering it would put an over-100% bar on screen.
  it("discards counters that disagree with their total", () => {
    const view = itemProgressView(tr, progress({
      completed_chunks: 900,
      total_chunks: 10,
    }))!;
    expect(view.chunks).toBeNull();
    expect(view.isUnknown).toBe(true);
  });

  it("discards negative and non-integer counters", () => {
    expect(itemProgressView(tr, progress({ completed_chunks: -1, total_chunks: 10 }))!.chunks).toBeNull();
    expect(itemProgressView(tr, progress({ completed_chunks: 1.5, total_chunks: 10 }))!.chunks).toBeNull();
  });

  // An empty document produces a real 0/0, which is finished work rather than
  // an indeterminate state.
  it("treats a zero total as complete", () => {
    const view = itemProgressView(tr, progress({
      completed_chunks: 0,
      total_chunks: 0,
      completed_batches: 0,
      total_batches: 0,
    }))!;
    expect(view.isComplete).toBe(true);
    expect(progressPercent(view.chunks)).toBeNull();
  });

  it("reports completion when the known totals are met", () => {
    const view = itemProgressView(tr, progress({
      completed_chunks: 512,
      total_chunks: 512,
      completed_batches: 16,
      total_batches: 16,
    }))!;
    expect(view.isComplete).toBe(true);
  });

  it("does not report completion while work remains", () => {
    const view = itemProgressView(tr, progress({
      completed_chunks: 70,
      total_chunks: 512,
    }))!;
    expect(view.isComplete).toBe(false);
  });

  // Docling Serve exposes a task status and no page counts, so the capability
  // copy belongs to the Docling stage only.
  it("shows Docling capability copy only during the Docling stage", () => {
    expect(itemProgressView(tr, progress({ stage: "docling", remote_status: "started" }))!.showsPageProgressUnavailable).toBe(true);
    for (const stage of ["storage", "parsing", "chunking", "indexing", "finalizing"]) {
      expect(itemProgressView(tr, progress({ stage }))!.showsPageProgressUnavailable).toBe(false);
    }
  });

  it("reads the remote status as an observation, never as a fraction", () => {
    const view = itemProgressView(tr, progress({
      stage: "docling",
      remote_status: "started",
    }))!;
    expect(view.remoteStatus).toBe("started");
    expect(view.chunks).toBeNull();
    expect(progressPercent(view.chunks)).toBeNull();
  });

  it("keeps the last-progress time as an observation", () => {
    const view = itemProgressView(tr, progress({ last_progress_at: "2026-07-20T00:02:00Z" }))!;
    expect(view.lastProgressAt).toBe("2026-07-20T00:02:00Z");
  });

  it("returns null when an item has no progress at all", () => {
    expect(itemProgressView(tr, null)).toBeNull();
    expect(itemProgressView(tr, undefined)).toBeNull();
  });
});