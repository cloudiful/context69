import { expect, test } from "@playwright/test";

import { ADMIN_USER, QUEUE_ACTIVE_TASK, QUEUE_FAILED_TASK } from "./fixtures";
import { installAppTransport, recordExternalRequests } from "./transport";

const queue = {
  processing: [QUEUE_ACTIVE_TASK, QUEUE_FAILED_TASK],
};

test.describe("react processing queue", () => {
  test("lists the processing rows and narrows them from the status header", async ({ page }) => {
    const external = recordExternalRequests(page);
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");

    await expect(page.getByRole("heading", { name: "Processing Queue", level: 1 })).toBeVisible();
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    const activeRow = page.getByTestId("queue-task-row").first();
    await expect(activeRow.getByText("Field Notes")).toBeVisible();
    const activeFileName = activeRow.getByText("field-notes.pdf");
    await expect(activeFileName).toBeVisible();
    await expect(activeFileName).toHaveClass(/text-muted-foreground/);
    await expect(page.getByTestId("queue-progress").first()).toContainText("2/4");
    await expect(page.getByTestId("queue-error").nth(1)).toContainText("Embedding provider timed out");

    await page.getByRole("button", { name: "Filter by Status" }).click();
    await page.getByRole("menuitem", { name: "Failed" }).click();

    await expect(page.getByTestId("queue-task-row")).toHaveCount(1);
    await expect(page.getByText("policy-review.pdf")).toBeVisible();
    expect(transport.calls.some((call) => (
      call.method === "GET" && call.path === "/v1/tasks" && call.query.includes("status=failed")
    ))).toBe(true);

    await page.getByLabel("Search files, groups, or errors").fill("policy");
    await page.getByLabel("Search files, groups, or errors").press("Enter");
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "GET" && call.path === "/v1/tasks" && call.query.includes("query=policy")
    )).length).toBeGreaterThan(0);
    await expect(page.getByTestId("queue-task-row")).toHaveCount(1);
    expect(external).toEqual([]);
    expect(transport.unmocked).toEqual([]);
  });

  test("expands items with truthful progress and reads diagnose facts", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    await page.getByRole("button", { name: "Expand task items" }).nth(1).click();
    await expect(page.getByText("policy-appendix.pdf")).toBeVisible();
    const progress = page.getByRole("progressbar", { name: "Indexing" });
    await expect(progress).toHaveAttribute("aria-valuenow", "40");
    await expect(page.getByText("4 / 10 chunks")).toBeVisible();

    await page.getByRole("button", { name: "Troubleshooting" }).click();
    await expect(page.getByText("Parent and item state disagree")).toBeVisible();
    await expect(page.getByText("Dependency gates")).toBeVisible();
    await expect(page.getByText("Half-open")).toBeVisible();
    await expect(page.getByText("Read-only troubleshooting")).toBeVisible();

    // A per-item `retryable: false` never hides the manual task-scoped action.
    await page.getByRole("button", { name: "Retry this task" }).first().click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-failed/retry"
    )).length).toBe(1);
  });

  test("confirms trash, then restores and permanently deletes from trash", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    // Only a terminal row offers the trash action: a running task must be
    // cancelled first, so the failed row is the one that can be trashed here.
    await page.getByRole("button", { name: "Move to trash" }).click();
    const trashDialog = page.getByRole("alertdialog");
    await expect(trashDialog).toContainText("Move this task history to the trash?");
    await trashDialog.getByRole("button", { name: "Move to trash" }).click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-failed/trash"
    )).length).toBe(1);

    await page.getByRole("tab", { name: "Trash" }).click();
    await expect(page.getByText("old-draft.pdf")).toBeVisible();

    await page.getByRole("button", { name: "Restore task" }).click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-trashed/restore"
    )).length).toBe(1);

    await page.getByRole("button", { name: "Delete permanently" }).click();
    const deleteDialog = page.getByRole("alertdialog");
    await expect(deleteDialog).toContainText("Permanently delete this task history?");
    await deleteDialog.getByRole("button", { name: "Delete" }).click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "DELETE" && call.path === "/v1/tasks/task-queue-trashed"
    )).length).toBe(1);
  });

  test("selects the current page and keeps the selection across pages", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    await expect(page.getByTestId("queue-selection-count")).toHaveText("0 selected");

    await page.getByRole("checkbox", { name: "Select Field Notes" }).check();
    await expect(page.getByTestId("queue-selection-count")).toHaveText("1 selected");

    // Sorting keeps the captured ids; only a filter change drops them.
    await page.getByRole("button", { name: "Sort by Created, sorted descending" }).click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "GET" && call.path === "/v1/tasks" && call.query.includes("sort_by=created_at&sort_direction=asc")
    )).length).toBeGreaterThan(0);
    await expect(page.getByTestId("queue-selection-count")).toHaveText("1 selected");

    await page.getByRole("button", { name: "Filter by Status" }).click();
    await page.getByRole("menuitem", { name: "Failed" }).click();
    await expect(page.getByTestId("queue-selection-count")).toHaveText("0 selected");
    expect(transport.unmocked).toEqual([]);
  });

  test("captures every matching task with the explicit select-all control", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      // Two rows on the page, three matching rows behind the filters.
      queue: {
        processing: [QUEUE_ACTIVE_TASK, QUEUE_FAILED_TASK, { ...QUEUE_FAILED_TASK, task_id: "task-queue-extra", file_name: "extra.pdf" }],
      },
    });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(3);

    await page.getByRole("button", { name: "Select all matching" }).click();
    await expect(page.getByTestId("queue-selection-count")).toHaveText("3 selected");
    await expect(page.getByTestId("queue-selection-snapshot")).toBeVisible();
    expect(transport.calls.some((call) => call.method === "GET" && call.path === "/v1/tasks/selection")).toBe(true);

    // Only the captured ids are targeted: the retry hits the failed rows, never
    // the active one.
    await page.getByTestId("queue-bulk-actions").getByRole("button", { name: "Retry" }).click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-failed/retry"
    )).length).toBe(1);
    expect(transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-active/retry"
    ))).toEqual([]);
  });

  test("fails closed when the matching set exceeds the snapshot cap", async ({ page }) => {
    await installAppTransport(page, {
      user: ADMIN_USER,
      queue: {
        selectionError: {
          status: 400,
          message: "more than 50000 tasks match the current filters; narrow the filters to capture a selection",
        },
      },
    });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    await page.getByRole("button", { name: "Select all matching" }).click();
    await expect(page.getByText("Failed to capture the matching tasks")).toBeVisible();
    await expect(page.getByTestId("queue-selection-count")).toHaveText("0 selected");
    await expect(page.getByTestId("queue-bulk-actions")).toHaveCount(0);
  });

  test("cancels then trashes a selected active task and confirms the count", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    await page.getByRole("checkbox", { name: "Select Field Notes" }).check();
    await page.getByTestId("queue-bulk-actions").getByRole("button", { name: "Move to trash" }).click();
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("Move 1 selected tasks to the trash?");
    await dialog.getByRole("button", { name: "Move to trash" }).click();

    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-active/trash"
    )).length).toBe(1);
    const paths = transport.calls.map((call) => `${call.method} ${call.path}`);
    expect(paths.indexOf("POST /v1/tasks/task-queue-active/cancel"))
      .toBeLessThan(paths.indexOf("POST /v1/tasks/task-queue-active/trash"));
  });

  test("permanently deletes a selected trash set only after the counted confirmation", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await page.getByRole("tab", { name: "Trash" }).click();
    await expect(page.getByText("old-draft.pdf")).toBeVisible();

    await page.getByRole("checkbox", { name: "Select the tasks on this page" }).check();
    await page.getByTestId("queue-bulk-actions").getByRole("button", { name: "Delete permanently" }).click();
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("Permanently delete 1 selected tasks?");
    await expect(dialog).toContainText("cannot be undone");
    await dialog.getByRole("button", { name: "Delete permanently" }).click();

    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "DELETE" && call.path === "/v1/tasks/task-queue-trashed"
    )).length).toBe(1);
    await expect(page.getByTestId("queue-selection-count")).toHaveText("0 selected");
  });

  test("freezes the view, search and filters while a bulk action runs", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      queue: { ...queue, actionDelayMs: 900 },
    });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    await page.getByRole("checkbox", { name: "Select Policy Review" }).check();
    await page.getByTestId("queue-bulk-actions").getByRole("button", { name: "Retry" }).click();

    // The captured ids must not be orphaned by a window change mid-batch.
    await expect(page.getByRole("tab", { name: "Trash" })).toBeDisabled();
    await expect(page.getByLabel("Search files, groups, or errors")).toBeDisabled();
    await expect(page.getByRole("button", { name: "Filter by Status" })).toBeDisabled();

    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "POST" && call.path === "/v1/tasks/task-queue-failed/retry"
    )).length).toBe(1);
    await expect(page.getByRole("tab", { name: "Trash" })).toBeEnabled();
  });

  test("removes the legacy top bulk commands", async ({ page }) => {
    await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    for (const name of [
      /Retry recoverable tasks/,
      "Cancel active",
      "Clear completed",
      "Empty trash",
    ]) {
      await expect(page.getByRole("button", { name })).toHaveCount(0);
    }

    await page.getByRole("tab", { name: "Completed" }).click();
    await expect(page.getByText("annual-report.pdf")).toBeVisible();
    await expect(page.getByRole("button", { name: "Clear completed" })).toHaveCount(0);
  });

  test("refreshes expanded item rows when the parent task turns terminal", async ({ page }) => {
    const terminal = {
      ...QUEUE_ACTIVE_TASK,
      status: "failed" as const,
      stage: "finalize",
      failure_stage: "indexing",
      error_summary: "Extraction failed",
      updated_at: "2026-01-01T08:05:30Z",
    };
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      queue: {
        processing: [QUEUE_ACTIVE_TASK],
        listSequence: [[QUEUE_ACTIVE_TASK], [terminal]],
        itemSequences: {
          [QUEUE_ACTIVE_TASK.task_id]: [
            {
              items: [{
                item_id: "queue-item-live",
                ordinal: 1,
                status: "running",
                attempt_count: 1,
                retryable: false,
                file_name: "field-notes.pdf",
                created_at: "2026-01-01T08:00:00Z",
              }],
              next_cursor: null,
            },
            {
              items: [{
                item_id: "queue-item-live",
                ordinal: 1,
                status: "failed",
                attempt_count: 1,
                retryable: false,
                file_name: "field-notes.pdf",
                error_message: "Extraction failed",
                created_at: "2026-01-01T08:00:00Z",
              }],
              next_cursor: null,
            },
          ],
        },
        streamUpdates: { delayMs: 900, tasks: [terminal] },
      },
    });
    const itemReads = () => transport.calls.filter((call) => (
      call.method === "GET" && call.path.endsWith("/items")
    )).length;

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toContainText("Running");
    await page.getByRole("button", { name: "Expand task items" }).click();
    await expect(page.getByTestId("task-item-row")).toContainText("Running");
    const readsBefore = itemReads();

    // The live frame changes the sort field, so the window reloads with the
    // terminal parent instead of patching it in place.
    await expect(page.getByTestId("queue-task-row")).toContainText("Failed", { timeout: 4_000 });

    // The item rows follow the parent revision promptly, without waiting for
    // the 3s active poll, and reload only the loaded extent (one page).
    await expect(page.getByTestId("task-item-row")).toContainText("Extraction failed", { timeout: 1_500 });
    const readsAfterTerminal = itemReads();
    expect(readsAfterTerminal).toBe(readsBefore + 1);

    // A terminal parent stops the periodic refresh.
    await page.waitForTimeout(3_500);
    expect(itemReads()).toBe(readsAfterTerminal);
  });

  test("shows readable task and item titles with distinct filenames", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      queue: {
        processing: [QUEUE_ACTIVE_TASK],
        itemSequences: {
          [QUEUE_ACTIVE_TASK.task_id]: [{
            items: [{
              item_id: "queue-item-titled",
              ordinal: 1,
              status: "running",
              attempt_count: 1,
              retryable: false,
              file_name: "2026-10-15-0099.PDF",
              document_title: "Quarterly Filing",
              created_at: "2026-01-01T08:00:00Z",
            }],
            next_cursor: null,
          }],
        },
      },
    });

    await page.goto("/processing-queue");
    const row = page.getByTestId("queue-task-row").first();
    await expect(row.getByText("Field Notes")).toBeVisible();
    await expect(row.getByText("field-notes.pdf")).toHaveClass(/text-muted-foreground/);

    await page.getByRole("button", { name: "Expand task items" }).click();
    const itemRow = page.getByTestId("task-item-row");
    await expect(itemRow.getByText("Quarterly Filing")).toBeVisible();
    await expect(itemRow.getByText("2026-10-15-0099.PDF")).toHaveClass(/text-muted-foreground/);
    expect(transport.unmocked).toEqual([]);
  });

  test("keeps a filtered window when the live snapshot arrives", async ({ page }) => {
    await installAppTransport(page, {
      user: ADMIN_USER,
      queue: { ...queue, stream: "snapshot" },
    });

    await page.goto("/processing-queue");
    await page.getByRole("button", { name: "Filter by Status" }).click();
    await page.getByRole("menuitem", { name: "Failed" }).click();
    await expect(page.getByTestId("queue-task-row")).toHaveCount(1);

    // The watch-all snapshot carries the processing window, including the
    // active task the filter excludes; it must not replace this window.
    await page.waitForTimeout(500);
    await expect(page.getByTestId("queue-task-row")).toHaveCount(1);
    await expect(page.getByText("policy-review.pdf")).toBeVisible();
    await expect(page.getByText("field-notes.pdf")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Filter by Status: Failed" })).toBeVisible();
  });

  test("keeps the refreshed progress when a buffered live frame flushes later", async ({ page }) => {
    const stale = {
      ...QUEUE_ACTIVE_TASK,
      progress: { total: 4, queued: 0, running: 2, waiting: 0, succeeded: 2, failed: 0, cancelled: 0 },
    };
    const current = {
      ...QUEUE_ACTIVE_TASK,
      progress: { total: 4, queued: 0, running: 1, waiting: 0, succeeded: 3, failed: 0, cancelled: 0 },
    };
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      queue: {
        processing: [stale],
        // The first read is the older state; any later authoritative read is current.
        listSequence: [[stale], [current]],
        streamUpdates: { delayMs: 300, tasks: [stale], updates: [stale] },
      },
    });

    await page.goto("/processing-queue");
    const progress = page.getByTestId("queue-progress").first();
    await expect(progress).toBeVisible();
    await expect(progress).toContainText("3/4");

    // The coalesced patch window closes after the authoritative read landed; the
    // buffered frame is older and must not roll the row back to 2/4.
    await page.waitForTimeout(400);
    await expect(progress).toContainText("3/4");
    await expect(progress).not.toContainText("2/4");

    const listReads = transport.calls.filter((call) => call.method === "GET" && call.path === "/v1/tasks");
    expect(listReads.length).toBeLessThanOrEqual(3);
  });

  test("fails a hung list at the deadline and loads it on the next Refresh", async ({ page }) => {
    await page.clock.install();
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      // Keep the live stream pending so only the explicit reads are in play.
      queue: { hangListAfter: 0, streamUpdates: { delayMs: 600_000, tasks: [] } },
    });

    await page.goto("/processing-queue");
    await expect(page.getByText("Loading")).toBeVisible();
    await expect(page.getByTestId("queue-task-row")).toHaveCount(0);

    // Freeze virtual time where the page currently is, then cross the deadline.
    await page.clock.pauseAt(new Date(Date.now()));
    await page.clock.runFor(16_000);

    await expect(page.getByText("Failed to load the processing queue")).toBeVisible();
    await expect(page.getByText("Queue list request timed out")).toBeVisible();

    // A manual refresh starts a new read and recovers. The clock resumes so the
    // recovery runs on normal timers instead of the frozen virtual one.
    await page.clock.resume();
    await page.getByRole("button", { name: "Refresh queue" }).click();
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    await expect(page.getByText("Failed to load the processing queue")).toHaveCount(0);
    const listReads = transport.calls.filter((call) => (
      call.method === "GET" && call.path === "/v1/tasks" && !call.query.includes("kind=")
    )).length;
    expect(listReads).toBe(2);
  });

  test("keeps the loaded rows when a Refresh times out", async ({ page }) => {
    await page.clock.install();
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      queue: { hangListAfter: 1, streamUpdates: { delayMs: 600_000, tasks: [] } },
    });

    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    await page.clock.pauseAt(new Date(Date.now()));

    await page.getByRole("button", { name: "Refresh queue" }).click();
    await expect(page.getByText("Loading")).toHaveCount(0);
    await page.clock.runFor(16_000);

    await expect(page.getByText("Failed to load the processing queue")).toBeVisible();
    // The stale-but-valid window stays on screen while recovery is offered.
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    await expect(page.getByText("field-notes.pdf")).toBeVisible();

    await page.clock.resume();
    await page.getByRole("button", { name: "Refresh queue" }).click();
    await expect(page.getByText("Failed to load the processing queue")).toHaveCount(0);
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    expect(transport.unmocked).toEqual([]);
  });

  test("renders English/light/desktop and Chinese/dark/narrow", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.addInitScript(() => {
      if (!window.localStorage.getItem("context69.locale")) {
        window.localStorage.setItem("context69.locale", "en");
        window.localStorage.setItem("context69.theme", "light");
      }
    });
    await installAppTransport(page, { user: ADMIN_USER, queue });

    await page.goto("/processing-queue");
    await expect(page.getByRole("tab", { name: "Processing" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Completed" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Trash" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(() => {
      window.localStorage.setItem("context69.locale", "zh-CN");
      window.localStorage.setItem("context69.theme", "dark");
    });
    await page.reload();

    await expect(page.getByRole("tab", { name: "处理中" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "已完成" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "回收站" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);
    const zhRow = page.getByTestId("queue-task-row").first();
    await expect(zhRow.getByText("Field Notes")).toBeVisible();
    await expect(zhRow.getByText("field-notes.pdf")).toBeVisible();
    await expect(page.getByRole("button", { name: "展开任务项" })).toHaveCount(2);
  });

  test("renders one header label with independent sort and filter actions at both widths", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, queue });
    await page.goto("/processing-queue");
    await expect(page.getByTestId("queue-task-row")).toHaveCount(2);

    const table = page.getByTestId("processing-queue-table");
    await expect(table.getByText("Type", { exact: true })).toHaveCount(1);
    await expect(table.getByText("Status", { exact: true })).toHaveCount(1);
    await expect(table.getByText("Stage", { exact: true })).toHaveCount(1);
    await expect(table.getByText("Waiting", { exact: true })).toHaveCount(1);

    const sortByType = table.getByRole("button", { name: "Sort by Type, not sorted" });
    const filterByType = table.getByRole("button", { name: "Filter by Type" });
    const listReads = () => transport.calls.filter((call) => call.method === "GET" && call.path === "/v1/tasks");

    const readsBefore = listReads().length;
    await filterByType.click();
    await expect(page.getByRole("menu")).toBeVisible();
    expect(listReads()).toHaveLength(readsBefore);
    await page.keyboard.press("Escape");

    await sortByType.click();
    await expect.poll(() => listReads().at(-1)?.query).toContain("sort_by=kind");
    await expect(table.getByRole("button", { name: "Sort by Type, sorted ascending" })).toHaveAttribute("data-sort", "asc");
    await expect(table.getByRole("button", { name: "Filter by Type" })).toBeVisible();

    for (const width of [1280, 600]) {
      await page.setViewportSize({ width, height: 900 });
      const heights = await table.locator("[data-slot='column-header']").evaluateAll(
        (nodes) => nodes.map((node) => node.getBoundingClientRect().height),
      );
      expect(heights.length).toBeGreaterThan(0);
      expect(heights.every((height) => Math.abs(height - 32) < 1)).toBe(true);
    }
  });
});
