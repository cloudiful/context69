// Truthful, stage-local progress rendering rules (issue 52).
//
// The generated `TaskItemProgress` carries what the worker actually observed:
// a stage, chunk and batch counters that may be individually unknown, an
// optional remote converter status, and the time progress was last seen. Every
// rule here exists so the UI cannot say more than that record supports — in
// particular it never renders a percentage for an unknown total, never invents
// a page fraction, and never calls a stalled task stalled.
//
// The type is derived from the generated schema rather than re-declared, so a
// contract change surfaces here as a type error instead of a silent drift.
import type { TaskItemResponse } from "../../services/api";

export type TaskItemProgress = NonNullable<TaskItemResponse["progress"]>;

type Translate = (key: string, params?: Record<string, unknown>) => string;

// The stages an item actually performs. This is deliberately NOT the persisted
// stage-column vocabulary: those values (`download`, `sync`, `processing`, …)
// drive the queue's stage filter, while these drive progress copy. Coupling
// them would make the filter offer options no persisted stage ever takes.
export const PIPELINE_PROGRESS_STAGES = [
  "storage",
  "parsing",
  "docling",
  "chunking",
  "indexing",
  "finalizing",
] as const;

export type PipelineProgressStage = (typeof PIPELINE_PROGRESS_STAGES)[number];

const PROGRESS_STAGE_SET: ReadonlySet<string> = new Set(PIPELINE_PROGRESS_STAGES);

// A stage this build does not know falls back to the Unknown label rather than
// leaking a raw identifier or a raw `processingQueue.progressStageLabels.*` key.
export function pipelineProgressStageLabel(t: Translate, stage: string | null | undefined): string {
  if (!stage || !PROGRESS_STAGE_SET.has(stage)) return t("processingQueue.unknownStage");
  return t(`processingQueue.progressStageLabels.${stage}`);
}

export interface ProgressFraction {
  completed: number;
  total: number;
}

/** A counter pair that is safe to show, or `null` when it is not. */
export type ProgressCount = ProgressFraction | null;

function readCount(value: number | null | undefined, label: string): number | null {
  if (value == null || !Number.isFinite(value)) return null;
  if (!Number.isInteger(value) || value < 0) return null;
  void label;
  return value;
}

// A pair is renderable only when the server reported both ends. A completed
// count with no total is a real observation, but there is nothing to make a
// fraction of, so it stays out of the bar and the caller shows it as a plain
// count. A completed count larger than its total is not a believable
// observation, so it is discarded rather than shown as an over-100% bar.
function readFraction(
  completed: number | null | undefined,
  total: number | null | undefined,
): ProgressCount {
  const done = readCount(completed, "completed");
  const whole = readCount(total, "total");
  if (done === null || whole === null) return null;
  if (done > whole) return null;
  return { completed: done, total: whole };
}

export interface ItemProgressView {
  /** Localized stage label; Unknown for a stage this build does not know. */
  stageLabel: string;
  /** The raw stage, so Docling capability copy can key off it. */
  stage: string | null;
  chunks: ProgressCount;
  batches: ProgressCount;
  /** True when the run has known totals and nothing is left to do. */
  isComplete: boolean;
  /** True when the record carries no counter this build can render. */
  isUnknown: boolean;
  remoteStatus: string | null;
  /** Only during a Docling stage, which exposes no page counts. */
  showsPageProgressUnavailable: boolean;
  lastProgressAt: string | null;
}

export function itemProgressView(
  t: Translate,
  progress: TaskItemProgress | null | undefined,
): ItemProgressView | null {
  if (!progress) return null;
  const stage = typeof progress.stage === "string" ? progress.stage : null;
  const chunks = readFraction(progress.completed_chunks, progress.total_chunks);
  const batches = readFraction(progress.completed_batches, progress.total_batches);
  const lastProgressAt = typeof progress.last_progress_at === "string" ? progress.last_progress_at : null;
  const remoteStatus = typeof progress.remote_status === "string" && progress.remote_status
    ? progress.remote_status
    : null;
  const isDocling = stage === "docling";
  return {
    stageLabel: pipelineProgressStageLabel(t, stage),
    stage,
    chunks,
    batches,
    // A known total that is already met is a finished run. `0/0` counts: an
    // empty document is genuinely complete, not indeterminate.
    isComplete: (chunks !== null && chunks.completed === chunks.total)
      || (batches !== null && batches.completed === batches.total && chunks === null),
    isUnknown: chunks === null && batches === null,
    remoteStatus,
    showsPageProgressUnavailable: isDocling,
    lastProgressAt,
  };
}

// The bar's percentage, or `null` when no fraction is justified. Only a known
// total greater than zero yields a number: an unknown total has no denominator
// and a zero total is already complete, so neither may be turned into a percent.
export function progressPercent(count: ProgressCount): number | null {
  if (!count || count.total <= 0) return null;
  return Math.round((count.completed / count.total) * 100);
}

// The `completed / total` text. Units stay explicit so a chunk count is never
// read as a batch count.
export function progressCountText(
  t: Translate,
  count: ProgressCount,
  unit: "chunks" | "batches",
): string | null {
  if (!count) return null;
  return t(`processingQueue.itemProgress.${unit}Count`, { completed: count.completed, total: count.total });
}