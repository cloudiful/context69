import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { apiClient } from "@/services/api";
import type { TaskPageResponse, TaskResponse } from "@/services/api";
import { formatTimestamp } from "@/lib/format";
import { installFakeEventSource } from "@/test-utils/event-source";

import { ProcessingQueuePage } from "./queue-page";

const auth = vi.hoisted(() => ({ admin: false }));
vi.mock("@/hooks/use-auth", () => ({
  useAuthSession: () => ({
    status: auth.admin ? "authenticated" : "guest",
    user: auth.admin
      ? {
          user_id: 1,
          login_name: "admin",
          display_name: "Admin",
          is_admin: true,
          disabled_at: null,
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
        }
      : null,
    ready: true,
    lastFailureReason: null,
    restoreError: null,
  }),
}));

function task(overrides: Partial<TaskResponse> = {}): TaskResponse {
  return {
    task_id: "task-1",
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: "research",
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    progress: { total: 4, queued: 0, running: 1, waiting: 0, succeeded: 3, failed: 0, cancelled: 0 },
    ...overrides,
  };
}

function page(items: TaskResponse[], overrides: Partial<TaskPageResponse["pagination"]> = {}): TaskPageResponse {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: 1, ...overrides },
  };
}

function LocationProbe() {
  const location = useLocation();
  return <span data-testid="location-search">{location.search}</span>;
}

function renderPage(path = "/processing-queue") {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <AppProviders>
        <ProcessingQueuePage />
        <LocationProbe />
      </AppProviders>
    </MemoryRouter>,
  );
}

describe("ProcessingQueuePage", () => {
  beforeEach(() => {
    auth.admin = false;
    installFakeEventSource();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("renders the processing rows with their status, stage and counters", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ file_name: "alpha.pdf", stage: "indexing" }),
    ]));
    renderPage();

    expect(await screen.findByText("alpha.pdf")).toBeInTheDocument();
    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.getByText("Indexing")).toBeInTheDocument();
    expect(screen.getByTestId("queue-progress")).toHaveTextContent("3/4");
    expect(screen.getByRole("tab", { name: "Processing" })).toHaveAttribute("data-state", "active");
  });

  it("keeps the legacy top bulk commands out of the header in every view", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockImplementation(async (params) => (
      params.view === "completed" ? page([]) : page([task()])
    ));
    renderPage();
    await screen.findAllByTestId("queue-task-row");

    for (const name of [
      "Retry recoverable tasks",
      "Cancel active",
      "Clear completed",
      "Empty trash",
    ]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument();
    }

    await user.click(screen.getByRole("tab", { name: "Completed" }));
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed" }),
      expect.anything(),
    ));
    expect(screen.queryByRole("button", { name: "Clear completed" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retry recoverable tasks" })).not.toBeInTheDocument();
    expect(await screen.findByText("No completed tasks")).toBeInTheDocument();
  });

  it("selects rows with checkboxes and offers contextual bulk actions", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "task-1", file_name: "alpha.pdf", status: "failed" }),
      task({ task_id: "task-2", file_name: "beta.pdf", status: "running" }),
    ]));
    renderPage();
    await screen.findByText("alpha.pdf");

    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("0 selected");
    expect(screen.queryByTestId("queue-bulk-actions")).not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select alpha.pdf" }));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("1 selected");
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Retry 1 failed" }))
      .toBeInTheDocument();
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Move 1 to trash" }))
      .toBeInTheDocument();
    expect(within(screen.getByTestId("queue-bulk-actions")).queryByRole("button", { name: /^Cancel/ }))
      .not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select beta.pdf" }));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("2 selected");
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Cancel 1" }))
      .toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear selection" }));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("0 selected");
  });

  it("offers the source rerun on a succeeded row and on a selection of successes", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "done-1", file_name: "alpha.pdf", status: "succeeded" }),
      task({ task_id: "done-2", file_name: "beta.pdf", status: "succeeded" }),
    ]));
    const rerun = vi.spyOn(apiClient, "rerunTask").mockResolvedValue({
      task: { task_id: "child-1", item_ids: [] },
    });
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(within(screen.getAllByTestId("queue-task-row")[0]!).getByRole("button", { name: "Reprocess source" }));
    await waitFor(() => expect(rerun).toHaveBeenCalledWith("done-1"));

    await user.click(screen.getByRole("checkbox", { name: "Select alpha.pdf" }));
    await user.click(screen.getByRole("checkbox", { name: "Select beta.pdf" }));
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Reprocess source 2" }))
      .toBeInTheDocument();
    expect(within(screen.getByTestId("queue-bulk-actions")).queryByRole("button", { name: /Retry/ }))
      .not.toBeInTheDocument();
  });

  it("disables view, search, filter and paging controls while a bulk action runs", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "failed-1", file_name: "alpha.pdf", status: "failed" }),
    ]));
    let release!: () => void;
    vi.spyOn(apiClient, "batchRetryTasks").mockImplementation(() => new Promise((resolve) => {
      release = () => resolve({
        results: [{ task_id: "failed-1", outcome: "accepted", retried_task_id: "failed-1" }],
      });
    }));
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("checkbox", { name: "Select alpha.pdf" }));
    await user.click(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Retry 1 failed" }));

    await waitFor(() => expect(screen.getByRole("tab", { name: "Trash" })).toBeDisabled());
    expect(screen.getByRole("tab", { name: "Completed" })).toBeDisabled();
    expect(screen.getByLabelText("Search files, groups, or errors")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Filter by Type" })).toBeDisabled();
    expect(screen.getByTestId("queue-pagination")).toHaveAttribute("inert");
    expect(screen.getByTestId("queue-pagination")).toHaveAttribute("aria-busy", "true");

    await act(async () => {
      release();
    });
    await waitFor(() => expect(screen.getByRole("tab", { name: "Trash" })).toBeEnabled());
  });

  it("selects the current page only from the header checkbox", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "task-1", file_name: "alpha.pdf", status: "failed" }),
      task({ task_id: "task-2", file_name: "beta.pdf", status: "failed" }),
    ]));
    const selection = vi.spyOn(apiClient, "listTaskSelection");
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("checkbox", { name: "Select the tasks on this page" }));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("2 selected");
    expect(selection).not.toHaveBeenCalled();
  });

  it("captures every matching task with the explicit select-all control", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "task-1", file_name: "alpha.pdf", status: "failed" }),
    ]));
    const selection = vi.spyOn(apiClient, "listTaskSelection").mockResolvedValue({
      items: [
        { task_id: "task-1", status: "failed" },
        { task_id: "task-9", status: "failed" },
      ],
      total: 2,
      limit: 50000,
    });
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("button", { name: "Select all matching" }));
    await waitFor(() => expect(selection).toHaveBeenCalledWith(expect.objectContaining({
      view: "processing",
      query: "",
    })));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("2 selected");
    expect(screen.getByTestId("queue-selection-snapshot")).toBeInTheDocument();
  });

  it("states the captured count before deleting a selection permanently", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "task-1", file_name: "alpha.pdf", status: "failed", deleted_at: "2026-01-02T00:00:00Z" }),
      task({ task_id: "task-2", file_name: "beta.pdf", status: "failed", deleted_at: "2026-01-02T00:00:00Z" }),
    ]));
    const remove = vi.spyOn(apiClient, "deleteTask").mockResolvedValue(undefined);
    renderPage();
    await screen.findByText("alpha.pdf");
    await user.click(screen.getByRole("tab", { name: "Trash" }));
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "trash" }),
      expect.anything(),
    ));

    await user.click(screen.getByRole("checkbox", { name: "Select the tasks on this page" }));
    await user.click(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Delete 2 permanently" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/2 selected tasks/)).toBeInTheDocument();
    expect(within(dialog).getByText(/cannot be undone/)).toBeInTheDocument();
    expect(remove).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(2));
    expect(remove).toHaveBeenCalledWith("task-1");
    expect(remove).toHaveBeenCalledWith("task-2");
  });

  it("trashes a failed task only after the confirmation dialog accepts", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf", status: "failed" })]));
    vi.spyOn(apiClient, "trashTask").mockResolvedValue(task({ status: "failed", deleted_at: "2026-01-02T00:00:00Z" }));
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("button", { name: "Move to trash" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/Move this task history to the trash/)).toBeInTheDocument();
    expect(apiClient.trashTask).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Move to trash" }));
    await waitFor(() => expect(apiClient.trashTask).toHaveBeenCalledWith("task-1"));
  });

  it("retries a failed row through the task-scoped endpoint", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf", status: "failed" })]));
    vi.spyOn(apiClient, "retryTask").mockResolvedValue({ retried_items: 2, task: { task_id: "task-1" } });
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("button", { name: "Retry failed items" }));
    await waitFor(() => expect(apiClient.retryTask).toHaveBeenCalledWith("task-1"));
  });

  it("shows a task title as the primary subject and a distinct filename secondary", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({
        file_name: "2026-10-15-0099.pdf",
        document_title: "Quarterly Filing",
        kind: "url_batch",
      }),
    ]));
    renderPage();

    const cell = await screen.findByTestId("queue-task");
    const primary = within(cell).getByText("Quarterly Filing");
    const secondary = within(cell).getByText("2026-10-15-0099.pdf");
    expect(primary).toBeInTheDocument();
    expect(secondary).toHaveClass("text-muted-foreground");
    expect(primary.compareDocumentPosition(secondary) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("does not repeat an identical title and filename", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ file_name: "same.pdf", document_title: "same.pdf" }),
    ]));
    renderPage();

    const cell = await screen.findByTestId("queue-task");
    expect(within(cell).getAllByText("same.pdf")).toHaveLength(1);
  });

  it("falls back to the filename when no title is available", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ file_name: "fallback.pdf" }),
    ]));
    renderPage();

    expect(await screen.findByText("fallback.pdf")).toBeInTheDocument();
  });

  it("expands a row into cursor-paged items", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf" })]));
    vi.spyOn(apiClient, "getTaskItems").mockResolvedValue({
      items: [{
        item_id: "item-1",
        ordinal: 1,
        status: "running",
        attempt_count: 1,
        retryable: true,
        created_at: "2026-01-01T00:00:00Z",
        file_name: "chapter-1.pdf",
        document_title: "Chapter One",
      }],
      next_cursor: null,
    });
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("button", { name: "Expand task items" }));
    expect(await screen.findByText("Chapter One")).toBeInTheDocument();
    expect(screen.getByText("chapter-1.pdf")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Troubleshooting" })).toBeInTheDocument();
  });

  it("shows the load failure with a retry path", async () => {
    vi.spyOn(apiClient, "listTasks").mockRejectedValue(new Error("queue unavailable"));
    renderPage();
    expect(await screen.findByText("Failed to load the processing queue")).toBeInTheDocument();
    expect(screen.getByText("queue unavailable")).toBeInTheDocument();
  });

  it("keeps the queue toggle disabled and offers a retry when the admission state is unknown", async () => {
    auth.admin = true;
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task()]));
    const getState = vi
      .spyOn(apiClient, "getTaskQueueState")
      .mockRejectedValueOnce(new Error("queue state unavailable"))
      .mockResolvedValueOnce({ paused: true, updated_at: "2026-01-01T00:00:00Z" });
    renderPage();

    expect(await screen.findByText("Failed to read the queue state")).toBeInTheDocument();
    expect(screen.getByTestId("queue-pause-toggle")).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: "Retry queue state" }));
    await waitFor(() => expect(getState).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: "Resume queue" })).toBeEnabled();
    expect(screen.queryByText("Failed to read the queue state")).not.toBeInTheDocument();
  });

  it("shows the trash-specific empty copy", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([]));
    renderPage();
    await waitFor(() => expect(screen.getByText("No tasks")).toBeInTheDocument());
    await user.click(screen.getByRole("tab", { name: "Trash" }));
    expect(await screen.findByText("Trash is empty")).toBeInTheDocument();
  });

  it("renders one visible label per column with independent sort and filter actions", async () => {
    const user = userEvent.setup();
    const list = vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf" })]));
    renderPage();
    await screen.findByText("alpha.pdf");

    const table = screen.getByTestId("processing-queue-table");
    expect(within(table).getAllByText("Type", { exact: true })).toHaveLength(1);
    expect(within(table).getAllByText("Status", { exact: true })).toHaveLength(1);
    expect(within(table).getAllByText("Stage", { exact: true })).toHaveLength(1);
    expect(within(table).getAllByText("Waiting", { exact: true })).toHaveLength(1);

    const sortByType = within(table).getByRole("button", { name: "Sort by Type, not sorted" });
    const filterByType = within(table).getByRole("button", { name: "Filter by Type" });
    expect(sortByType).toHaveAttribute("data-sort", "none");
    expect(filterByType).toHaveAttribute("data-filter-active", "false");

    const callsBefore = list.mock.calls.length;
    await user.click(filterByType);
    expect(await screen.findByRole("menu")).toBeInTheDocument();
    expect(list.mock.calls).toHaveLength(callsBefore);
    await user.keyboard("{Escape}");

    await user.click(sortByType);
    await waitFor(() => expect(list).toHaveBeenLastCalledWith(
      expect.objectContaining({ sortBy: "kind", sortDirection: "asc" }),
      expect.anything(),
    ));
    expect(within(table).getByRole("button", { name: "Sort by Type, sorted ascending" })).toHaveAttribute("data-sort", "asc");
  });

  it("marks the sorted column for assistive technology", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf" })]));
    renderPage();
    await screen.findByText("alpha.pdf");

    const createdHead = screen.getByText("Created", { exact: true }).closest("th");
    expect(createdHead).toHaveAttribute("aria-sort", "descending");
  });

  it("shows the persisted finish time in the completed view without inventing a sort field", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockImplementation(async (params) => (
      params.view === "completed"
        ? page([task({
            file_name: "done.pdf",
            status: "succeeded",
            updated_at: "2026-02-03T04:00:00Z",
            finished_at: "2026-02-03T04:05:06Z",
          })])
        : page([task({ file_name: "alpha.pdf" })])
    ));
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("tab", { name: "Completed" }));

    expect(await screen.findByText(formatTimestamp("2026-02-03T04:05:06Z"))).toBeInTheDocument();
    const table = screen.getByTestId("processing-queue-table");
    const finishedHead = within(table).getByText("Completed", { exact: true }).closest("th");
    expect(finishedHead).not.toHaveAttribute("aria-sort");
    expect(within(finishedHead as HTMLElement).queryByRole("button")).not.toBeInTheDocument();
  });

  it("keeps a bulk report with the accepted target and opens it", async () => {
    const user = userEvent.setup();
    const taskId = "11111111-1111-4111-8111-111111111111";
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: taskId, file_name: "alpha.pdf", status: "failed" }),
    ]));
    vi.spyOn(apiClient, "batchRetryTasks").mockResolvedValue({
      results: [{ task_id: taskId, outcome: "accepted", retried_task_id: taskId }],
    });
    vi.spyOn(apiClient, "getTask").mockResolvedValue(task({
      task_id: taskId,
      file_name: "alpha.pdf",
      status: "queued",
    }));
    vi.spyOn(apiClient, "getTaskItems").mockResolvedValue({ items: [], next_cursor: null });
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("checkbox", { name: "Select alpha.pdf" }));
    await user.click(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Retry 1 failed" }));

    const report = await screen.findByTestId("queue-bulk-report");
    expect(screen.getByTestId("queue-bulk-report-summary"))
      .toHaveTextContent("1 accepted · 0 skipped · 0 failed");
    const row = within(report).getByTestId("queue-bulk-report-row");
    expect(row).toHaveTextContent("alpha.pdf");
    expect(row).toHaveTextContent("Accepted");

    await user.click(within(row).getByRole("button", { name: "Open alpha.pdf" }));
    expect(screen.getByTestId("location-search")).toHaveTextContent(`task=${taskId}`);
    expect(await screen.findByText("Focused task")).toBeInTheDocument();
    await waitFor(() => expect(apiClient.getTask).toHaveBeenCalledWith(taskId, expect.anything()));

    await user.click(screen.getByRole("button", { name: "Dismiss result" }));
    expect(screen.queryByTestId("queue-bulk-report")).not.toBeInTheDocument();
  });

  it("shows one focused task even when it is outside the current page", async () => {
    const taskId = "22222222-2222-4222-8222-222222222222";
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([]));
    vi.spyOn(apiClient, "getTask").mockResolvedValue(task({
      task_id: taskId,
      file_name: "off-page.pdf",
      status: "queued",
    }));
    vi.spyOn(apiClient, "getTaskItems").mockResolvedValue({ items: [], next_cursor: null });
    renderPage(`/processing-queue?task=${taskId}&keep=1`);

    const focused = await screen.findByTestId("queue-focused-task");
    expect(await within(focused).findByText("off-page.pdf")).toBeInTheDocument();
    expect(within(focused).getByText("Queued")).toBeInTheDocument();
    expect(apiClient.getTask).toHaveBeenCalledTimes(1);

    await userEvent.setup().click(screen.getByRole("button", { name: "Close task view" }));
    expect(screen.getByTestId("location-search")).toHaveTextContent("?keep=1");
  });

  it("rejects a malformed focus link without a request", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf" })]));
    const getTask = vi.spyOn(apiClient, "getTask");
    renderPage("/processing-queue?task=not-a-uuid");

    expect(screen.getByTestId("queue-focused-task")).toHaveTextContent("This task link is not valid.");
    expect(getTask).not.toHaveBeenCalled();
  });

  it("explains an unavailable focused task without leaking its state", async () => {
    const taskId = "33333333-3333-4333-8333-333333333333";
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ file_name: "alpha.pdf" })]));
    vi.spyOn(apiClient, "getTask").mockRejectedValue(
      Object.assign(new Error("forbidden"), { status: 403 }),
    );
    renderPage(`/processing-queue?task=${taskId}`);

    expect(await screen.findByTestId("queue-focused-task-error"))
      .toHaveTextContent("This task is unavailable or you do not have access to it.");
  });
});
