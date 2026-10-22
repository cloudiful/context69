import { mkdir, readFile, writeFile } from "node:fs/promises";
import { cpus, homedir, platform, release, totalmem } from "node:os";
import { dirname, isAbsolute, join, resolve, sep } from "node:path";
import type { Page } from "@playwright/test";
import type { QueueMockTransport } from "./transport";

const APPROVED_ROOT = resolve(homedir(), ".cache/opencode/context69-queue-perf/1014");
export const QUEUE_PERF_BASE_URL = "http://127.0.0.1:5188";

export interface QueuePerfSettings {
  label: string;
  baseSha: string;
  outputDir: string;
}

interface BrowserProbeSnapshot {
  durationMs: number;
  longTaskDurationsMs: number[];
  frameGapsMs: number[];
  inputEventLatencyMs: number[];
  tableMutationCallbacks: number;
  tableMutationRecords: number;
}

function distribution(values: number[]) {
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = (fraction: number) => sorted.length === 0
    ? null
    : sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
  return {
    samples: values,
    count: values.length,
    p50: percentile(0.5),
    p95: percentile(0.95),
    max: sorted.length ? sorted[sorted.length - 1] : null,
  };
}

export function queuePerfSettings(): QueuePerfSettings {
  const label = process.env.QUEUE_PERF_LABEL ?? "";
  const baseSha = process.env.QUEUE_PERF_BASE_SHA ?? "";
  const configuredOutput = process.env.QUEUE_PERF_OUTPUT ?? "";
  if (!/^[A-Za-z0-9._-]{1,48}$/.test(label)) {
    throw new Error("Set QUEUE_PERF_LABEL to a short run label such as baseline or optimized.");
  }
  if (!/^[0-9a-f]{7,40}$/i.test(baseSha)) {
    throw new Error("Set QUEUE_PERF_BASE_SHA to the source revision used for this run.");
  }
  if (!configuredOutput || !isAbsolute(configuredOutput)) {
    throw new Error("Set QUEUE_PERF_OUTPUT to a unique absolute directory under the issue cache.");
  }
  const outputDir = resolve(configuredOutput);
  if (!outputDir.startsWith(`${APPROVED_ROOT}${sep}`)) {
    throw new Error(`QUEUE_PERF_OUTPUT must be inside ${APPROVED_ROOT}.`);
  }
  return { label, baseSha, outputDir };
}

export function installQueuePerformanceProbe() {
  const state = {
    startedAt: performance.now(),
    longTaskDurationsMs: [] as number[],
    frameGapsMs: [] as number[],
    inputEventLatencyMs: [] as number[],
    tableMutationCallbacks: 0,
    tableMutationRecords: 0,
  };
  let lastFrameAt = performance.now();
  let observedTable: Element | null = null;
  let tableObserver: MutationObserver | null = null;

  try {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (state.longTaskDurationsMs.length < 2_000) state.longTaskDurationsMs.push(entry.duration);
      }
    }).observe({ type: "longtask", buffered: true });
  } catch {
    // Long-task timing is optional on browsers that do not expose the entry type.
  }

  function frame(now: number) {
    if (state.frameGapsMs.length < 20_000) state.frameGapsMs.push(now - lastFrameAt);
    lastFrameAt = now;
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  document.addEventListener("click", (event) => {
    if (state.inputEventLatencyMs.length < 2_000) {
      state.inputEventLatencyMs.push(Math.max(0, performance.now() - event.timeStamp));
    }
  }, true);

  const probe = {
    attachTable() {
      const table = document.querySelector('[data-testid="processing-queue-table"]');
      if (!table || table === observedTable) return;
      tableObserver?.disconnect();
      observedTable = table;
      tableObserver = new MutationObserver((records) => {
        state.tableMutationCallbacks += 1;
        state.tableMutationRecords += records.length;
      });
      tableObserver.observe(table, { attributes: true, characterData: true, childList: true, subtree: true });
    },
    reset() {
      this.attachTable();
      state.startedAt = performance.now();
      lastFrameAt = state.startedAt;
      state.longTaskDurationsMs.length = 0;
      state.frameGapsMs.length = 0;
      state.inputEventLatencyMs.length = 0;
      state.tableMutationCallbacks = 0;
      state.tableMutationRecords = 0;
    },
    read(): BrowserProbeSnapshot {
      this.attachTable();
      return {
        durationMs: performance.now() - state.startedAt,
        longTaskDurationsMs: [...state.longTaskDurationsMs],
        frameGapsMs: [...state.frameGapsMs],
        inputEventLatencyMs: [...state.inputEventLatencyMs],
        tableMutationCallbacks: state.tableMutationCallbacks,
        tableMutationRecords: state.tableMutationRecords,
      };
    },
  };
  Object.defineProperty(window, "__queuePerfProbe", { configurable: true, value: probe });
}

export async function resetQueuePerformanceProbe(page: Page): Promise<void> {
  await page.evaluate(() => {
    (window as unknown as { __queuePerfProbe?: { reset: () => void } }).__queuePerfProbe?.reset();
  });
}

export async function collectScenarioMetrics(
  page: Page,
  transport: QueueMockTransport,
  name: string,
  observations: Record<string, unknown> = {},
) {
  const [probe, dom, stream] = await Promise.all([
    page.evaluate(() => (window as unknown as {
      __queuePerfProbe?: { read: () => BrowserProbeSnapshot };
    }).__queuePerfProbe?.read() ?? null),
    page.evaluate(() => {
      const table = document.querySelector('[data-testid="processing-queue-table"]');
      const body = document.body?.innerText ?? "";
      return {
        taskRows: table?.querySelectorAll('[data-testid="queue-task"]').length ?? 0,
        tableRows: table?.querySelectorAll("tbody tr").length ?? 0,
        cells: table?.querySelectorAll("td").length ?? 0,
        expandedRows: document.querySelectorAll('[data-testid="task-items-count"]').length,
        expandedItems: document.querySelectorAll('[data-testid="task-item-row"]').length,
        renderedInvalidValues: body.includes("undefined") || body.includes("NaN"),
      };
    }),
    page.evaluate(() => {
      const registry = (window as unknown as {
        __queueEventSource?: { instances: unknown[]; updatesEmitted: number };
      }).__queueEventSource;
      return { connections: registry?.instances.length ?? 0, updatesEmitted: registry?.updatesEmitted ?? 0 };
    }),
  ]);
  if (!probe) throw new Error("Queue performance probe was not installed in the browser.");
  const requests = transport.snapshot();
  return {
    name,
    capturedAt: new Date().toISOString(),
    durationMs: probe.durationMs,
    longTasks: distribution(probe.longTaskDurationsMs),
    animationFrameGaps: distribution(probe.frameGapsMs),
    inputEventLatency: distribution(probe.inputEventLatencyMs),
    tableMutations: {
      callbacks: probe.tableMutationCallbacks,
      records: probe.tableMutationRecords,
    },
    dom,
    requests,
    stream,
    observations,
  };
}

export function queuePerfEnvironment(browserVersion: string) {
  const cpu = cpus()[0];
  return {
    browser: { engine: "chromium", version: browserVersion },
    os: { platform: platform(), release: release() },
    hardware: {
      cpuModel: cpu?.model ?? "unknown",
      logicalCpuCount: cpus().length,
      totalMemoryBytes: totalmem(),
    },
    nodeVersion: process.version,
    viewport: { width: 1440, height: 1000 },
  };
}

export async function saveQueuePerfRun(
  settings: QueuePerfSettings,
  browserVersion: string,
  scenarios: Array<Record<string, unknown>>,
): Promise<string> {
  const output = join(settings.outputDir, "metrics.json");
  await mkdir(dirname(output), { recursive: true });
  let priorScenarios: Array<Record<string, unknown>> = [];
  try {
    const existing = JSON.parse(await readFile(output, "utf8")) as {
      label: string;
      baseSha: string;
      scenarios: Array<Record<string, unknown>>;
    };
    if (existing.label !== settings.label || existing.baseSha !== settings.baseSha) {
      throw new Error(`Queue performance output belongs to another run: ${output}.`);
    }
    priorScenarios = existing.scenarios;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  await writeFile(output, `${JSON.stringify({
    label: settings.label,
    baseSha: settings.baseSha,
    environment: queuePerfEnvironment(browserVersion),
    scenarios: [...priorScenarios, ...scenarios],
  }, null, 2)}\n`, "utf8");
  return output;
}
