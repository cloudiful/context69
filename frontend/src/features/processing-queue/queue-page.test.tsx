import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { apiClient } from "@/services/api";
import type { TaskPageResponse, TaskResponse } from "@/services/api";
import { formatTimestamp } from "@/lib/format";
import { installFakeEventSource } from "@/test-utils/event-source";

import { ProcessingQueuePage } from "./queue-page";

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

function renderPage() {
  return render(
    <AppProviders>
      <ProcessingQueuePage />
    </AppProviders>,
  );
}

describe("ProcessingQueuePage", () => {
  beforeEach(() => {
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
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Retry" }))
      .toBeInTheDocument();
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Move to trash" }))
      .toBeInTheDocument();
    expect(within(screen.getByTestId("queue-bulk-actions")).queryByRole("button", { name: "Cancel" }))
      .not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select beta.pdf" }));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("2 selected");
    expect(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Cancel" }))
      .toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear selection" }));
    expect(screen.getByTestId("queue-selection-count")).toHaveTextContent("0 selected");
  });

  it("disables view, search, filter and paging controls while a bulk action runs", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([
      task({ task_id: "failed-1", file_name: "alpha.pdf", status: "failed" }),
    ]));
    let release!: () => void;
    vi.spyOn(apiClient, "retryTask").mockImplementation(() => new Promise((resolve) => {
      release = () => resolve({ retried_items: 1, task: { task_id: "failed-1" } });
    }));
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("checkbox", { name: "Select alpha.pdf" }));
    await user.click(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Retry" }));

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
    await user.click(within(screen.getByTestId("queue-bulk-actions")).getByRole("button", { name: "Delete permanently" }));
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

    await user.click(screen.getByRole("button", { name: "Retry this task" }));
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
});
