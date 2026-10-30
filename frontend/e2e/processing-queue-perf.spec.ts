import { expect, test, type Browser, type Page } from "@playwright/test";
import type { components } from "../src/generated/openapi";
import { makeQueueTasks } from "./queue-perf/fixtures";
import {
  collectScenarioMetrics,
  installQueuePerformanceProbe,
  QUEUE_PERF_BASE_URL,
  queuePerfSettings,
  resetQueuePerformanceProbe,
  saveQueuePerfRun,
} from "./queue-perf/metrics";
import { QueueMockTransport } from "./queue-perf/transport";

type TaskResponse = components["schemas"]["TaskResponse"];
type TaskStatus = components["schemas"]["TaskStatus"];

const settings = queuePerfSettings();
const scenarios: Array<Record<string, unknown>> = [];
const VIEWPORT = { width: 1440, height: 1000 };
interface QueuePage {
  page: Page;
  transport: QueueMockTransport;
  tasks: TaskResponse[];
}

async function runWithQueuePage(
  browser: Browser,
  statuses: (index: number) => TaskStatus,
  options: ConstructorParameters<typeof QueueMockTransport>[1],
  run: (queue: QueuePage) => Promise<void>,
): Promise<void> {
  const tasks = makeQueueTasks(100, statuses);
  const context = await browser.newContext({ viewport: VIEWPORT, serviceWorkers: "block" });
  const page = await context.newPage();
  const transport = new QueueMockTransport(tasks, options);
  await context.addInitScript(installQueuePerformanceProbe);
  try {
    await transport.install(context, QUEUE_PERF_BASE_URL);
    await page.goto(`${QUEUE_PERF_BASE_URL}/processing-queue`);
    await expect(page.getByTestId("processing-queue-table")).toBeVisible();
    await expect(page.getByTestId("queue-task")).toHaveCount(25);
    const pageSize = page.getByRole("combobox", { name: "Items per page" });
    await pageSize.click();
    await page.getByRole("option", { name: "100", exact: true }).click();
    await expect(page.getByTestId("queue-task")).toHaveCount(100);
    await run({ page, transport, tasks });
  } finally {
    const unmatchedApiRequests = transport.snapshot().unmatchedApiRequests;
    await context.close();
    expect(unmatchedApiRequests, "every API endpoint must have an explicit fixture").toEqual([]);
  }
}

async function recordScenario(
  queue: QueuePage,
  name: string,
  observations: Record<string, unknown> = {},
) {
  const result = await collectScenarioMetrics(queue.page, queue.transport, name, observations);
  scenarios.push(result);
  return result;
}

async function emitUpdates(
  page: Page,
  tasks: TaskResponse[],
  count: number,
  intervalMs: number,
): Promise<{ elapsedMs: number; firstToLastMs: number }> {
  return page.evaluate(async ({ sourceTasks, updateCount, delayMs }) => {
    const registry = (window as unknown as {
      __queueEventSource?: { emit: (type: string, payload: unknown) => void };
    }).__queueEventSource;
    if (!registry) throw new Error("Synthetic EventSource registry is unavailable.");
    const startedAt = performance.now();
    let firstEventAt = startedAt;
    let lastEventAt = startedAt;
    for (let index = 0; index < updateCount; index += 1) {
      const task = sourceTasks[index % sourceTasks.length];
      const updated = {
        ...task,
        updated_at: new Date(Date.parse(task.updated_at) + index + 1).toISOString(),
        progress: { ...task.progress, succeeded: (index % 199) + 1 },
      };
      lastEventAt = performance.now();
      if (index === 0) firstEventAt = lastEventAt;
      registry.emit("update", { task: updated });
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    return { elapsedMs: performance.now() - startedAt, firstToLastMs: lastEventAt - firstEventAt };
  }, { sourceTasks: tasks, updateCount: count, delayMs: intervalMs });
}

test.afterAll(async ({ browser }) => {
  const output = await saveQueuePerfRun(settings, browser.version(), scenarios);
  console.info(`Queue performance results: ${output}`);
});

test("records the 100-row, zero-expansion table baseline", async ({ browser }) => {
  await runWithQueuePage(browser, () => "running", {}, async (queue) => {
    await resetQueuePerformanceProbe(queue.page);
    await queue.page.waitForTimeout(250);
    const measured = await recordScenario(queue, "100-rows-zero-expanded", {
      workload: "baseline",
      expectedVisibleTaskRows: 100,
      expectedExpandedRows: 0,
    });
    expect(measured.dom.taskRows).toBe(100);
    expect(measured.dom.expandedRows).toBe(0);
  });
});

test("captures 100 updates at 5ms and a separate 1000-update stress burst", async ({ browser }) => {
  test.setTimeout(360_000);
  await runWithQueuePage(browser, () => "running", {}, async (queue) => {
    const stableListRequests = queue.transport.snapshot().listRequests;
    await resetQueuePerformanceProbe(queue.page);
    const burst = emitUpdates(queue.page, queue.tasks, 100, 5);
    await queue.page.waitForTimeout(100);
    await queue.page.getByPlaceholder("Search files, groups, or errors").click();
    const normalEmission = await burst;
    await queue.page.waitForTimeout(1_100);
    const normal = await recordScenario(queue, "100-updates-5ms", {
      workload: "synthetic-normal-window",
      requestedUpdates: 100,
      requestedIntervalMs: 5,
      emissionElapsedMs: normalEmission.elapsedMs,
      actualMeanInterarrivalMs: normalEmission.firstToLastMs / 99,
      listRequestsBefore: stableListRequests,
      listRequestsAfter: queue.transport.snapshot().listRequests,
      clickInjectedDuringBurst: true,
    });
    expect(normal.stream.updatesEmitted).toBe(100);
    expect(normal.requests.listRequests).toBe(stableListRequests);

    await resetQueuePerformanceProbe(queue.page);
    const listRequestsBeforeStress = queue.transport.snapshot().listRequests;
    const stressBurst = emitUpdates(queue.page, queue.tasks, 1_000, 1);
    await queue.page.waitForTimeout(100);
    await queue.page.getByPlaceholder("Search files, groups, or errors").click();
    const stressEmission = await stressBurst;
    await queue.page.waitForTimeout(1_100);
    const stress = await recordScenario(queue, "1000-updates-1ms-stress", {
      workload: "synthetic-stress-beyond-normal-server-coalescing",
      requestedUpdates: 1_000,
      requestedIntervalMs: 1,
      emissionElapsedMs: stressEmission.elapsedMs,
      actualMeanInterarrivalMs: stressEmission.firstToLastMs / 999,
      cumulativeUpdates: 1_100,
      listRequestsBefore: listRequestsBeforeStress,
      listRequestsAfter: queue.transport.snapshot().listRequests,
      clickInjectedDuringBurst: true,
    });
    expect(stress.stream.updatesEmitted).toBe(1_100);
    expect(stress.dom.taskRows).toBe(100);
    expect(stress.requests.listRequests).toBe(listRequestsBeforeStress);
  });
});

test("captures one and ten expanded rows after loading two item pages each", async ({ browser }) => {
  await runWithQueuePage(browser, () => "failed", {}, async (queue) => {
    await resetQueuePerformanceProbe(queue.page);
    await queue.page.getByRole("button", { name: "Expand task items" }).first().click();
    await expect(queue.page.getByTestId("task-item-row")).toHaveCount(100);
    await queue.page.getByTestId("task-items-load-more").first().click();
    await expect(queue.page.getByTestId("task-item-row")).toHaveCount(200);
    await queue.page.waitForTimeout(250);
    const oneExpanded = await recordScenario(queue, "1-expanded-2-pages", {
      workload: "expanded-row-fanout",
      expandedTasks: 1,
      loadedPagesPerTask: 2,
      loadedItemsPerTask: 200,
    });
    expect(oneExpanded.dom.expandedRows).toBe(1);
    expect(oneExpanded.dom.expandedItems).toBe(200);

    await resetQueuePerformanceProbe(queue.page);
    for (let index = 1; index < 10; index += 1) {
      await queue.page.getByRole("button", { name: "Expand task items" }).first().click();
    }
    await expect(queue.page.getByTestId("task-items-count")).toHaveCount(10);
    await expect(queue.page.getByTestId("task-item-row")).toHaveCount(1_100);
    for (let index = 0; index < 9; index += 1) {
      const loadMore = queue.page.getByTestId("task-items-load-more").first();
      const before = await queue.page.getByTestId("task-items-load-more").count();
      await loadMore.click();
      await expect(queue.page.getByTestId("task-items-load-more")).toHaveCount(before - 1);
    }
    await expect(queue.page.getByTestId("task-item-row")).toHaveCount(2_000);
    await queue.page.waitForTimeout(250);
    const tenExpanded = await recordScenario(queue, "10-expanded-2-pages-each", {
      workload: "expanded-row-fanout",
      expandedTasks: 10,
      loadedPagesPerTask: 2,
      loadedItemsPerTask: 200,
    });
    expect(tenExpanded.dom.expandedRows).toBe(10);
    expect(tenExpanded.dom.expandedItems).toBe(2_000);
  });
});

test("records mocked 100-task bulk retry and resume latency and summary", async ({ browser }) => {
  const failTaskId = "00000000-0000-4000-8000-000000000100";
  await runWithQueuePage(
    browser,
    (index) => index % 2 === 0 ? "failed" : "cancelled",
    { actionDelayMs: 25, failActionTaskId: failTaskId },
    async (queue) => {
      await resetQueuePerformanceProbe(queue.page);
      // One captured snapshot of the whole filtered set, then the contextual
      // retry of the failed rows only.
      await queue.page.getByRole("button", { name: "Select all matching" }).click();
      await expect(queue.page.getByTestId("queue-selection-count")).toHaveText("100 selected");
      await queue.page.getByTestId("queue-bulk-actions").getByRole("button", { name: "Retry" }).click();
      await expect(queue.page.getByText("50 succeeded · 0 skipped · 0 failed", { exact: true })).toBeVisible();

      // The cancelled rows stay selected, so the resume targets exactly them.
      await expect(queue.page.getByTestId("queue-selection-count")).toHaveText("50 selected");
      await queue.page.getByTestId("queue-bulk-actions").getByRole("button", { name: "Resume" }).click();
      await expect(queue.page.getByText("49 succeeded · 0 skipped · 1 failed", { exact: true })).toBeVisible();
      const pageText = await queue.page.locator("body").innerText();
      const toastLines = pageText.split("\n").map((line) => line.trim()).filter((line) =>
        /succeeded.*skipped.*failed/.test(line),
      );
      const measured = await recordScenario(queue, "100-bulk-recovery-mixed", {
        workload: "mocked-bulk-selection",
        requestedTargets: 100,
        actionDelayMs: 25,
        expectedSuccessfulRetries: 50,
        expectedSuccessfulReruns: 49,
        remainingFailedAfterReload: 1,
        observedToastLines: toastLines,
      });
      expect(measured.requests.selectionRequests).toBe(1);
      expect(measured.requests.actionRequests).toHaveLength(100);
      expect(measured.requests.retryRequests).toBe(50);
      expect(measured.requests.rerunRequests).toBe(50);
      // The bulk runner is bounded to four in-flight submissions.
      expect(measured.requests.maximumActionConcurrency).toBeGreaterThan(0);
      expect(measured.requests.maximumActionConcurrency).toBeLessThanOrEqual(4);
    },
  );
});
