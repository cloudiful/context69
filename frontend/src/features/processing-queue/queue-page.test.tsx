import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { apiClient } from "@/services/api";
import type { TaskPageResponse, TaskResponse } from "@/services/api";
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

  it("switches to the completed view with clear history instead of bulk recovery", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "listTasks").mockImplementation(async (params) => (
      params.view === "completed" ? page([]) : page([task()])
    ));
    renderPage();
    await screen.findAllByTestId("queue-task-row");

    await user.click(screen.getByRole("tab", { name: "Completed" }));
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed" }),
      expect.anything(),
    ));
    expect(screen.getByRole("button", { name: "Clear completed" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retry recoverable tasks" })).not.toBeInTheDocument();
    expect(await screen.findByText("No completed tasks")).toBeInTheDocument();
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
      }],
      next_cursor: null,
    });
    renderPage();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("button", { name: "Expand task items" }));
    expect(await screen.findByText("chapter-1.pdf")).toBeInTheDocument();
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
});
