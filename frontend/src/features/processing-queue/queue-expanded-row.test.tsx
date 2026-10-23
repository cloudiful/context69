import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { i18n } from "@/lib/i18n";
import { apiClient } from "@/services/api";
import type { TaskDiagnoseResponse, TaskItemResponse, TaskItemsResponse, TaskResponse } from "@/services/api";

import { QueueExpandedRow } from "./queue-expanded-row";

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
    progress: { total: 5, queued: 2, running: 1, waiting: 0, succeeded: 2, failed: 0, cancelled: 0 },
    ...overrides,
  };
}

function item(overrides: Partial<TaskItemResponse> = {}): TaskItemResponse {
  return {
    item_id: "item-1",
    ordinal: 1,
    status: "running",
    attempt_count: 1,
    retryable: true,
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function itemsResponse(items: TaskItemResponse[], nextCursor: string | null = null): TaskItemsResponse {
  return { items, next_cursor: nextCursor };
}

function diagnoseResponse(overrides: Partial<TaskDiagnoseResponse> = {}): TaskDiagnoseResponse {
  return {
    consistency: {
      consistent: true,
      mismatches: [],
      open_attempt_count: 2,
      near_exhaustion_item_count: 0,
    },
    dependency_gates: [],
    items: [],
    items_truncated: false,
    observed_at: "2026-01-01T00:00:00Z",
    task: {
      task_id: "task-1",
      kind: "file_batch",
      status: "failed",
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      progress: { total: 5, queued: 0, running: 0, waiting: 0, succeeded: 4, failed: 1, cancelled: 0 },
    },
    ...overrides,
  };
}

function renderRow(props: Partial<{ task: TaskResponse; active: boolean }> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onRetry = vi.fn();
  const element = (nextTask: TaskResponse, active: boolean) => (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={client}>
        <QueueExpandedRow
          task={nextTask}
          active={active}
          isActing={false}
          onRetry={onRetry}
        />
      </QueryClientProvider>
    </I18nextProvider>
  );
  const initialTask = props.task ?? task();
  const initialActive = props.active ?? false;
  const view = render(element(initialTask, initialActive));
  return {
    onRetry,
    show(nextTask: TaskResponse, active: boolean) {
      view.rerender(element(nextTask, active));
    },
  };
}

describe("QueueExpandedRow", () => {
  beforeEach(() => {
    vi.spyOn(apiClient, "getTaskItems").mockResolvedValue(itemsResponse([
      item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf" }),
    ], "cursor-2"));
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("loads the first page and appends the next cursor page without dropping rows", async () => {
    const user = userEvent.setup();
    const { onRetry } = renderRow();
    expect(await screen.findByText("alpha.pdf")).toBeInTheDocument();
    expect(screen.getByText("Showing 1 / 5")).toBeInTheDocument();
    expect(onRetry).not.toHaveBeenCalled();

    vi.mocked(apiClient.getTaskItems).mockImplementation(async (_taskId, params) => (
      params.cursor === "cursor-2"
        ? itemsResponse([item({ item_id: "item-2", ordinal: 2, file_name: "beta.pdf" })], null)
        : itemsResponse([item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf" })], "cursor-2")
    ));

    await user.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("beta.pdf")).toBeInTheDocument();
    expect(screen.getByText("alpha.pdf")).toBeInTheDocument();
    expect(apiClient.getTaskItems).toHaveBeenLastCalledWith(
      "task-1",
      expect.objectContaining({ limit: 100, cursor: "cursor-2" }),
      expect.anything(),
    );
  });

  it("shows an item title as primary with a distinct filename secondary", async () => {
    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({
        item_id: "item-1",
        file_name: "2026-10-15-0099.pdf",
        document_title: "Quarterly Filing",
      }),
    ]));
    renderRow();

    const row = await screen.findByTestId("task-item-row");
    const primary = within(row).getByText("Quarterly Filing");
    const secondary = within(row).getByText("2026-10-15-0099.pdf");
    expect(secondary).toHaveClass("text-muted-foreground");
    expect(primary.compareDocumentPosition(secondary) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("does not repeat an identical item title and filename", async () => {
    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({ item_id: "item-1", file_name: "same.pdf", document_title: "same.pdf" }),
    ]));
    renderRow();

    const row = await screen.findByTestId("task-item-row");
    expect(within(row).getAllByText("same.pdf")).toHaveLength(1);
  });

  it("re-requests the first page with the item status filter", async () => {
    const user = userEvent.setup();
    Element.prototype.hasPointerCapture = () => false;
    Element.prototype.setPointerCapture = () => undefined;
    Element.prototype.releasePointerCapture = () => undefined;
    renderRow();
    await screen.findByText("alpha.pdf");

    await user.click(screen.getByRole("combobox", { name: "Item status" }));
    await user.click(await screen.findByRole("option", { name: "Failed" }));

    await waitFor(() => expect(apiClient.getTaskItems).toHaveBeenLastCalledWith(
      "task-1",
      expect.objectContaining({ status: "failed", limit: 100 }),
      expect.anything(),
    ));
    expect(apiClient.getTaskItems).toHaveBeenLastCalledWith(
      "task-1",
      expect.not.objectContaining({ cursor: "cursor-2" }),
      expect.anything(),
    );
  });

  it("keeps loaded rows when a later page fails", async () => {
    const user = userEvent.setup();
    renderRow();
    await screen.findByText("alpha.pdf");

    vi.mocked(apiClient.getTaskItems).mockRejectedValueOnce(new Error("network down"));
    await user.click(screen.getByRole("button", { name: "Load more" }));

    await waitFor(() => expect(apiClient.getTaskItems).toHaveBeenCalledTimes(2));
    expect(screen.getByText("alpha.pdf")).toBeInTheDocument();
    expect(screen.queryByText("Failed to load task items")).not.toBeInTheDocument();
  });

  it("loads diagnose only after the operator opens troubleshooting", async () => {
    const user = userEvent.setup();
    vi.spyOn(apiClient, "getTaskDiagnose").mockResolvedValue(diagnoseResponse());
    renderRow();
    await screen.findByText("alpha.pdf");
    expect(apiClient.getTaskDiagnose).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
    expect(await screen.findByText("2 open attempts")).toBeInTheDocument();
    expect(apiClient.getTaskDiagnose).toHaveBeenCalledWith("task-1", expect.anything());
    expect(screen.getByText(/Read-only troubleshooting/)).toBeInTheDocument();
  });

  it("offers the task-scoped retry from the task status, not the item retryable flag", async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf", status: "failed", retryable: false }),
    ]));
    const { onRetry } = renderRow({ task: task({ status: "failed" }) });

    await screen.findByText("alpha.pdf");
    const retry = screen.getByRole("button", { name: "Retry this task" });
    await user.click(retry);
    expect(onRetry).toHaveBeenCalledWith(expect.objectContaining({ task_id: "task-1" }));
  });

  it("hides the retry action while the task is active", async () => {
    renderRow({ task: task({ status: "running" }) });
    await screen.findByText("alpha.pdf");
    expect(screen.queryByRole("button", { name: "Retry this task" })).not.toBeInTheDocument();
  });

  it("renders truthful item progress without inventing a percentage or counts", async () => {
    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({
        item_id: "item-1",
        file_name: "alpha.pdf",
        progress: { attempt: 1, stage: "indexing", total_chunks: null, completed_chunks: null },
      }),
    ]));
    renderRow();
    await screen.findByText("alpha.pdf");
    expect(screen.getAllByText("Indexing")).not.toHaveLength(0);
    expect(screen.getByText("Working — no counts reported yet")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("reloads the loaded extent when the parent task turns terminal", async () => {
    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf", status: "running" }),
    ]));
    const { show } = renderRow({
      task: task({ status: "running", updated_at: "2026-01-01T00:00:00Z" }),
      active: true,
    });
    expect(await screen.findByText("alpha.pdf")).toBeInTheDocument();
    expect(apiClient.getTaskItems).toHaveBeenCalledTimes(1);

    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf", status: "succeeded" }),
    ]));
    show(task({ status: "succeeded", updated_at: "2026-01-01T00:00:07Z" }), false);

    await waitFor(() => expect(screen.getByTestId("task-item-row")).toHaveTextContent("Succeeded"));
    expect(apiClient.getTaskItems).toHaveBeenCalledTimes(2);

    // A terminal parent stops the periodic refresh: no request follows the reload.
    await new Promise((resolve) => setTimeout(resolve, 120));
    expect(apiClient.getTaskItems).toHaveBeenCalledTimes(2);
  });

  it("coalesces a burst of parent revisions into one reload of the loaded extent", async () => {
    vi.mocked(apiClient.getTaskItems).mockImplementation(async (_taskId, params) => (
      params.cursor === "cursor-2"
        ? itemsResponse([item({ item_id: "item-2", ordinal: 2, file_name: "beta.pdf" })], null)
        : itemsResponse([item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf" })], "cursor-2")
    ));
    const user = userEvent.setup();
    const { show } = renderRow({
      task: task({ status: "running", updated_at: "2026-01-01T00:00:00Z" }),
      active: true,
    });
    expect(await screen.findByText("alpha.pdf")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("beta.pdf")).toBeInTheDocument();
    // The first page plus the two requests that reload both loaded pages.
    expect(apiClient.getTaskItems).toHaveBeenCalledTimes(3);

    const deferred: { release?: (value: TaskItemsResponse) => void } = {};
    vi.mocked(apiClient.getTaskItems).mockImplementationOnce(() => new Promise((resolve) => {
      deferred.release = resolve;
    }));
    show(task({ status: "running", updated_at: "2026-01-01T00:00:03Z" }), true);
    await waitFor(() => expect(apiClient.getTaskItems).toHaveBeenCalledTimes(4));
    show(task({ status: "running", updated_at: "2026-01-01T00:00:05Z" }), true);

    deferred.release?.(
      itemsResponse([item({ item_id: "item-1", ordinal: 1, file_name: "alpha.pdf" })], "cursor-2"),
    );

    // The burst request reads both loaded pages (4 and 5), then exactly one
    // trailing reload of both pages (6 and 7) absorbs the second revision.
    await waitFor(() => expect(apiClient.getTaskItems).toHaveBeenCalledTimes(7));
    expect(apiClient.getTaskItems).toHaveBeenLastCalledWith(
      "task-1",
      expect.objectContaining({ cursor: "cursor-2" }),
      expect.anything(),
    );
    await new Promise((resolve) => setTimeout(resolve, 120));
    expect(apiClient.getTaskItems).toHaveBeenCalledTimes(7);
  });

  it("shows a real fraction only for a known non-zero total", async () => {
    vi.mocked(apiClient.getTaskItems).mockResolvedValue(itemsResponse([
      item({
        item_id: "item-1",
        file_name: "alpha.pdf",
        progress: { attempt: 1, stage: "indexing", total_chunks: 10, completed_chunks: 4 },
      }),
    ]));
    renderRow();
    await screen.findByText("alpha.pdf");
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
    expect(screen.getByText("4 / 10 chunks")).toBeInTheDocument();
  });
});
