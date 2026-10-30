import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { i18n } from "@/lib/i18n";
import { apiClient } from "@/services/api";
import type { TaskPageResponse, TaskResponse, TaskStatus } from "@/services/api";
import { FakeEventSource, installFakeEventSource } from "@/test-utils/event-source";

import { QUEUE_LIST_TIMEOUT_MS, useQueue } from "./use-queue";

const toast = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
}));
vi.mock("sonner", () => ({ toast }));

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
    progress: { total: 2, queued: 0, running: 1, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
    ...overrides,
  };
}

function page(items: TaskResponse[], overrides: Partial<TaskPageResponse["pagination"]> = {}): TaskPageResponse {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: 1, ...overrides },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function renderQueue() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
    },
  });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </I18nextProvider>
  );
  return renderHook(() => useQueue(), { wrapper });
}

describe("useQueue", () => {
  beforeEach(() => {
    installFakeEventSource();
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task()]));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    toast.success.mockReset();
    toast.error.mockReset();
    toast.warning.mockReset();
  });

  it("loads the processing view with the complete default scope", async () => {
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    expect(apiClient.listTasks).toHaveBeenCalledWith(
      expect.objectContaining({
        page: 1,
        pageSize: 25,
        view: "processing",
        query: "",
        status: null,
        kind: null,
        stage: null,
        waitingReason: null,
        dependencyKey: null,
        sortBy: "created_at",
        sortDirection: "desc",
      }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  it("re-requests with a narrowing status filter and resets to page one", async () => {
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.setStatus("failed");
    });
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "failed", page: 1 }),
      expect.anything(),
    ));

    act(() => {
      result.current.setPage(2);
    });
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "failed", page: 2 }),
      expect.anything(),
    ));
  });

  it("submits the trimmed search and keeps the typed view", async () => {
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.setSearchInput("  report  ");
    });
    act(() => {
      result.current.submitSearch();
    });
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "report", page: 1, view: "processing" }),
      expect.anything(),
    ));
  });

  it("never adopts the watch-all snapshot as the window and reloads authoritatively", async () => {
    const completedPage = page(
      [task({ task_id: "done-1", status: "succeeded", progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 } })],
      { total: 7 },
    );
    vi.mocked(apiClient.listTasks).mockResolvedValue(page(
      [task({ task_id: "page-2-task" })],
      { page: 2, total: 60, total_pages: 3 },
    ));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    act(() => {
      result.current.setPage(2);
    });
    await waitFor(() => expect(result.current.items[0]?.task_id).toBe("page-2-task"));

    const before = vi.mocked(apiClient.listTasks).mock.calls.length;
    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [task({ task_id: "other" }), completedPage.items[0]] });
    });

    // The snapshot rows are not inserted and the authoritative page/totals stay.
    expect(result.current.items.map((item) => item.task_id)).toEqual(["page-2-task"]);
    expect(result.current.pagination?.total).toBe(60);
    await waitFor(() => expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBeGreaterThan(before));
    expect(result.current.pagination?.total).toBe(60);
  });

  it("patches known rows from a progress frame without reloading", async () => {
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [] });
    });
    await waitFor(() => expect(result.current.live.connected).toBe(true));
    const calls = vi.mocked(apiClient.listTasks).mock.calls.length;

    act(() => {
      FakeEventSource.instances[0]!.emit("update", {
        task: task({
          progress: { total: 2, queued: 0, running: 0, waiting: 0, succeeded: 2, failed: 0, cancelled: 0 },
        }),
      });
    });

    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(2));
    expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBe(calls);
  });

  it("keeps the authoritative progress when a buffered frame flushes after the read lands", async () => {
    const authoritative = (succeeded: number) => task({
      progress: { total: 5, queued: 0, running: 0, waiting: 0, succeeded, failed: 0, cancelled: 0 },
    });
    // Any authoritative read beyond the first returns the newer server state.
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([authoritative(3)]));
    vi.mocked(apiClient.listTasks).mockResolvedValueOnce(page([authoritative(1)]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(1));
    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [] });
    });
    await waitFor(() => expect(result.current.live.connected).toBe(true));
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(3));
    const callsBefore = vi.mocked(apiClient.listTasks).mock.calls.length;

    // A progress frame is buffered for the 100ms coalescing window...
    act(() => {
      FakeEventSource.instances[0]!.emit("update", { task: authoritative(2) });
    });
    // ...and the operator's Refresh confirms the newer server progress.
    act(() => {
      result.current.refresh();
    });
    await waitFor(() => expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBeGreaterThan(callsBefore));

    // The delayed flush must not roll the row back to the stale frame.
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(result.current.items[0]?.progress.succeeded).toBe(3);
    expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBeLessThanOrEqual(callsBefore + 2);
  });

  it("applies a frame received after the authoritative landing", async () => {
    const authoritative = (succeeded: number) => task({
      progress: { total: 5, queued: 0, running: 0, waiting: 0, succeeded, failed: 0, cancelled: 0 },
    });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([authoritative(1)]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(1));
    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [] });
    });
    await waitFor(() => expect(result.current.live.connected).toBe(true));

    vi.mocked(apiClient.listTasks).mockResolvedValueOnce(page([authoritative(3)]));
    act(() => {
      result.current.refresh();
    });
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(3));

    act(() => {
      FakeEventSource.instances[0]!.emit("update", { task: authoritative(4) });
    });
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(4));
  });

  it("fences a frame received while an authoritative read is still pending", async () => {
    const authoritative = (succeeded: number) => task({
      progress: { total: 5, queued: 0, running: 0, waiting: 0, succeeded, failed: 0, cancelled: 0 },
    });
    const pending = deferred<TaskPageResponse>();
    vi.mocked(apiClient.listTasks)
      .mockResolvedValueOnce(page([authoritative(1)]))
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValue(page([authoritative(3)]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(1));
    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [] });
    });
    await waitFor(() => expect(result.current.live.connected).toBe(true));
    const callsBefore = vi.mocked(apiClient.listTasks).mock.calls.length;

    act(() => {
      result.current.refresh();
    });
    act(() => {
      FakeEventSource.instances[0]!.emit("update", { task: authoritative(2) });
    });
    await act(async () => {
      pending.resolve(page([authoritative(3)]));
    });

    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(3));
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(result.current.items[0]?.progress.succeeded).toBe(3);
    // The in-flight frame asked for exactly one trailing reload, never a loop.
    expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBeLessThanOrEqual(callsBefore + 2);
  });

  it("keeps a usable progress frame when the authoritative read fails", async () => {
    const authoritative = (succeeded: number) => task({
      progress: { total: 5, queued: 0, running: 0, waiting: 0, succeeded, failed: 0, cancelled: 0 },
    });
    vi.mocked(apiClient.listTasks).mockRejectedValue(new Error("list unavailable"));
    vi.mocked(apiClient.listTasks)
      .mockResolvedValueOnce(page([authoritative(1)]))
      .mockResolvedValueOnce(page([authoritative(1)]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items[0]?.progress.succeeded).toBe(1));
    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [] });
    });
    await waitFor(() => expect(result.current.live.connected).toBe(true));

    act(() => {
      FakeEventSource.instances[0]!.emit("update", { task: authoritative(2) });
    });
    act(() => {
      result.current.refresh();
    });
    await new Promise((resolve) => setTimeout(resolve, 250));

    // A failed read is not an authoritative landing: the frame still applies.
    expect(result.current.items[0]?.progress.succeeded).toBe(2);
  });

  it("single-flights reloads while a list request is in flight", async () => {
    const first = deferred<TaskPageResponse>();
    vi.mocked(apiClient.listTasks)
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue(page([task()]));
    const { result } = renderQueue();

    act(() => {
      const source = FakeEventSource.instances[0]!;
      source.emit("snapshot", { tasks: [] });
      source.emit("update", { task: task({ status: "succeeded" }) });
      source.emit("update", { task: task({ task_id: "task-2", status: "succeeded" }) });
      source.emit("update", { task: task({ task_id: "task-3", status: "succeeded" }) });
    });

    await act(async () => {
      first.resolve(page([task()]));
    });
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    await waitFor(() => expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBe(2));
    expect(vi.mocked(apiClient.listTasks).mock.calls.length).toBe(2);
  });

  it("retries a failed task and reruns a cancelled task in place", async () => {
    vi.spyOn(apiClient, "retryTask").mockResolvedValue({ retried_items: 1, task: { task_id: "failed-1" } });
    vi.spyOn(apiClient, "rerunTask").mockResolvedValue({ task: { task_id: "cancelled-1" } });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "failed-1", status: "failed" }),
      task({ task_id: "cancelled-1", status: "cancelled" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    await act(async () => {
      result.current.recoverTask(result.current.items[0]!);
      result.current.recoverTask(result.current.items[1]!);
    });

    await waitFor(() => expect(apiClient.retryTask).toHaveBeenCalledWith("failed-1"));
    expect(apiClient.rerunTask).toHaveBeenCalledWith("cancelled-1");
  });

  it("does not offer recovery for an active task and cancels it instead", async () => {
    vi.spyOn(apiClient, "cancelTask").mockResolvedValue(undefined);
    vi.spyOn(apiClient, "retryTask").mockResolvedValue({ retried_items: 1, task: { task_id: "task-1" } });
    vi.spyOn(apiClient, "rerunTask").mockResolvedValue({ task: { task_id: "task-1" } });
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    await act(async () => {
      result.current.recoverTask(result.current.items[0]!);
    });
    expect(apiClient.retryTask).not.toHaveBeenCalled();
    expect(apiClient.rerunTask).not.toHaveBeenCalled();

    await act(async () => {
      result.current.cancelTask(result.current.items[0]!);
    });
    await waitFor(() => expect(apiClient.cancelTask).toHaveBeenCalledWith("task-1"));
  });

  it("requires confirmation before trashing a terminal task", async () => {
    vi.spyOn(apiClient, "trashTask").mockResolvedValue(task({ status: "succeeded" }));
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ status: "succeeded", progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 } }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.confirmTrashTask(result.current.items[0]!);
    });
    expect(apiClient.trashTask).not.toHaveBeenCalled();
    expect(result.current.confirm?.title).toBeTruthy();

    await act(async () => {
      result.current.confirmAccept();
    });
    await waitFor(() => expect(apiClient.trashTask).toHaveBeenCalledWith("task-1"));
    expect(result.current.confirm).toBeNull();
  });

  it("restores without a prompt and deletes only after confirmation in trash", async () => {
    vi.spyOn(apiClient, "restoreTask").mockResolvedValue(task());
    vi.spyOn(apiClient, "deleteTask").mockResolvedValue(undefined);
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "trashed-1", status: "failed", deleted_at: "2026-01-02T00:00:00Z" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    act(() => {
      result.current.setView("trash");
    });
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "trash" }),
      expect.anything(),
    ));

    await act(async () => {
      result.current.restoreTask(result.current.items[0]!);
    });
    await waitFor(() => expect(apiClient.restoreTask).toHaveBeenCalledWith("trashed-1"));

    act(() => {
      result.current.confirmDeleteTask(result.current.items[0]!);
    });
    expect(apiClient.deleteTask).not.toHaveBeenCalled();
    await act(async () => {
      result.current.confirmAccept();
    });
    await waitFor(() => expect(apiClient.deleteTask).toHaveBeenCalledWith("trashed-1"));
  });

  it("retries the selected tasks only, bounded to four concurrent requests", async () => {
    const gates = Array.from({ length: 6 }, () => deferred<{ retried_items: number; task: { task_id: string } }>());
    let active = 0;
    let peak = 0;
    vi.spyOn(apiClient, "retryTask").mockImplementation(() => {
      const index = active;
      active += 1;
      peak = Math.max(peak, active);
      return gates[index]!.promise.finally(() => {
        active -= 1;
      });
    });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page(
      Array.from({ length: 6 }, (_, index) => task({ task_id: `failed-${index}`, status: "failed" })),
    ));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(6));

    act(() => {
      result.current.togglePageSelection();
    });
    expect(result.current.selectionCount).toBe(6);

    await act(async () => {
      result.current.runBulkAction("retry");
    });
    await waitFor(() => expect(apiClient.retryTask).toHaveBeenCalledTimes(4));
    expect(peak).toBeLessThanOrEqual(4);

    await act(async () => {
      gates.forEach((gate) => gate.resolve({ retried_items: 1, task: { task_id: "x" } }));
    });
    await waitFor(() => expect(apiClient.retryTask).toHaveBeenCalledTimes(6));
    expect(peak).toBeLessThanOrEqual(4);
    for (let index = 0; index < 6; index += 1) {
      expect(apiClient.retryTask).toHaveBeenCalledWith(`failed-${index}`);
    }
  });

  it("acts only on the selected ids and leaves the rest untouched", async () => {
    vi.spyOn(apiClient, "retryTask").mockResolvedValue({ retried_items: 1, task: { task_id: "failed-1" } });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "failed-1", status: "failed" }),
      task({ task_id: "failed-2", status: "failed" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    await act(async () => {
      result.current.runBulkAction("retry");
    });

    await waitFor(() => expect(apiClient.retryTask).toHaveBeenCalledTimes(1));
    expect(apiClient.retryTask).toHaveBeenCalledWith("failed-1");
    expect(apiClient.retryTask).not.toHaveBeenCalledWith("failed-2");
  });

  it("reports a skipped retry truthfully and keeps the task selected", async () => {
    vi.spyOn(apiClient, "retryTask").mockRejectedValue(
      Object.assign(new Error("task has no failed items to retry"), { status: 400 }),
    );
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ status: "failed" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    await act(async () => {
      result.current.runBulkAction("retry");
    });

    await waitFor(() => expect(toast.warning).toHaveBeenCalled());
    expect(toast.warning.mock.calls[0]?.[0]).toContain("1 skipped");
    expect(result.current.selectionCount).toBe(1);
  });

  it("clears the successful tasks and keeps the failed ones selected", async () => {
    vi.spyOn(apiClient, "retryTask").mockImplementation(async (taskId: string) => {
      if (taskId === "failed-2") {
        throw new Error("boom");
      }
      return { retried_items: 1, task: { task_id: taskId } };
    });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "failed-1", status: "failed" }),
      task({ task_id: "failed-2", status: "failed" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    act(() => {
      result.current.togglePageSelection();
    });
    await act(async () => {
      result.current.runBulkAction("retry");
    });

    await waitFor(() => expect(toast.warning).toHaveBeenCalled());
    expect(result.current.selection.ids).toEqual(["failed-2"]);
  });

  it("keeps the selection across paging and sorting and drops it on a filter change", async () => {
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "task-1", status: "failed" }),
      task({ task_id: "task-2", status: "failed" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    expect(result.current.selectionCount).toBe(1);

    act(() => {
      result.current.setPage(2);
    });
    expect(result.current.selectionCount).toBe(1);

    act(() => {
      result.current.setSort("updated_at", "desc");
    });
    expect(result.current.selectionCount).toBe(1);

    act(() => {
      result.current.setStatus("failed");
    });
    await waitFor(() => expect(result.current.selectionCount).toBe(0));
  });

  it("captures every matching task as a snapshot and selects only those ids", async () => {
    vi.spyOn(apiClient, "listTaskSelection").mockResolvedValue({
      items: [
        { task_id: "matched-1", status: "failed" },
        { task_id: "matched-2", status: "queued" },
      ],
      total: 2,
      limit: 50000,
    });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ task_id: "task-1", status: "failed" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.selectAllMatching();
    });

    await waitFor(() => expect(result.current.selectionCount).toBe(2));
    expect(apiClient.listTaskSelection).toHaveBeenCalledWith(expect.objectContaining({
      view: "processing",
      query: "",
    }));
    expect(result.current.selection.snapshot).toBe(true);
    expect(result.current.selection.ids).toEqual(["matched-1", "matched-2"]);
  });

  it("freezes the window and the selection while a bulk action runs", async () => {
    const gates = Array.from({ length: 2 }, () => deferred<{ retried_items: number; task: { task_id: string } }>());
    let call = 0;
    vi.spyOn(apiClient, "retryTask").mockImplementation(() => {
      const gate = gates[call];
      call += 1;
      return gate!.promise;
    });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "failed-1", status: "failed" }),
      task({ task_id: "failed-2", status: "failed" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    act(() => {
      result.current.togglePageSelection();
    });
    let running: Promise<void>;
    act(() => {
      result.current.runBulkAction("retry");
      running = Promise.resolve();
    });

    await waitFor(() => expect(result.current.selectionBusy).toBe(true));
    expect(result.current.bulkAction).toBe("retry");

    // Every window or selection mutator is refused while the batch runs.
    act(() => {
      result.current.setView("trash");
    });
    act(() => {
      result.current.setStatus("failed");
    });
    act(() => {
      result.current.setKind("url_batch");
    });
    act(() => {
      result.current.submitSearch();
    });
    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    act(() => {
      result.current.togglePageSelection();
    });
    act(() => {
      result.current.clearSelection();
    });
    act(() => {
      result.current.setPage(2);
    });
    act(() => {
      result.current.setPageSize(50);
    });

    expect(result.current.scope.view).toBe("processing");
    expect(result.current.scope.status).toBeNull();
    expect(result.current.scope.kind).toBeNull();
    expect(result.current.scope.page).toBe(1);
    expect(result.current.scope.pageSize).toBe(25);
    expect(result.current.selectionCount).toBe(2);

    await act(async () => {
      gates.forEach((gate) => gate.resolve({ retried_items: 1, task: { task_id: "x" } }));
      await running!;
    });
    await waitFor(() => expect(result.current.selectionBusy).toBe(false));
  });

  it("freezes the window while a snapshot is being captured", async () => {
    const pending = deferred<{
      items: { task_id: string; status: TaskStatus }[];
      total: number;
      limit: number;
    }>();
    vi.spyOn(apiClient, "listTaskSelection").mockReturnValue(pending.promise);
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ task_id: "task-1", status: "failed" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.selectAllMatching();
    });
    await waitFor(() => expect(result.current.selectionBusy).toBe(true));

    act(() => {
      result.current.setView("trash");
    });
    act(() => {
      result.current.setStatus("failed");
    });
    expect(result.current.scope.view).toBe("processing");
    expect(result.current.scope.status).toBeNull();

    await act(async () => {
      pending.resolve({ items: [{ task_id: "task-1", status: "failed" }], total: 1, limit: 50000 });
    });
    await waitFor(() => expect(result.current.selectionCount).toBe(1));
  });

  it("drops a snapshot answer for a window that has been left", async () => {
    const pending = deferred<{
      items: { task_id: string; status: TaskStatus }[];
      total: number;
      limit: number;
    }>();
    vi.spyOn(apiClient, "listTaskSelection").mockReturnValue(pending.promise);
    vi.spyOn(apiClient, "rerunTask").mockResolvedValue({ task: { task_id: "cancelled-1" } });
    vi.mocked(apiClient.listTasks).mockImplementation(async (params) => (
      params.status === "cancelled"
        ? page([task({ task_id: "cancelled-1", status: "cancelled" })])
        : page([task({ task_id: "cancelled-1", status: "queued" })])
    ));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    act(() => {
      result.current.setStatus("cancelled");
    });
    await waitFor(() => expect(result.current.scope.status).toBe("cancelled"));

    act(() => {
      result.current.selectAllMatching();
    });
    await waitFor(() => expect(result.current.selectionBusy).toBe(true));

    // A single-row recovery clears the narrowing status filter mid-capture, so
    // the window moves while the snapshot read is still in flight. Its answer
    // must not replace the (now empty) selection with ids of the old window.
    await act(async () => {
      result.current.recoverTask(result.current.items[0]!);
    });
    await waitFor(() => expect(result.current.scope.status).toBeNull());

    await act(async () => {
      pending.resolve({ items: [{ task_id: "ghost", status: "cancelled" }], total: 1, limit: 50000 });
    });
    await waitFor(() => expect(result.current.selectionBusy).toBe(false));
    expect(result.current.selection.ids).toEqual([]);
    expect(toast.error).toHaveBeenCalled();
  });

  it("reports a refused snapshot instead of selecting anything", async () => {
    vi.spyOn(apiClient, "listTaskSelection").mockRejectedValue(
      new Error("more than 50000 tasks match the current filters"),
    );
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.selectAllMatching();
    });

    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    expect(result.current.selectionCount).toBe(0);
  });

  it("cancels an active task before moving it to the trash", async () => {
    const order: string[] = [];
    vi.spyOn(apiClient, "cancelTask").mockImplementation(async () => {
      order.push("cancel");
      return undefined;
    });
    vi.spyOn(apiClient, "trashTask").mockImplementation(async () => {
      order.push("trash");
      return task({ status: "cancelled" });
    });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ task_id: "active-1", status: "running" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    act(() => {
      result.current.runBulkAction("trash");
    });
    expect(result.current.confirm).not.toBeNull();
    expect(result.current.confirm?.message).toContain("1");

    await act(async () => {
      result.current.confirmAccept();
    });
    await waitFor(() => expect(apiClient.trashTask).toHaveBeenCalledWith("active-1"));
    expect(order).toEqual(["cancel", "trash"]);
  });

  it("reports a cancelled task that could not be trashed as a partial failure", async () => {
    vi.spyOn(apiClient, "cancelTask").mockResolvedValue(undefined);
    vi.spyOn(apiClient, "trashTask").mockRejectedValue(new Error("trash unavailable"));
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ task_id: "active-1", status: "running" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    act(() => {
      result.current.runBulkAction("trash");
    });
    await act(async () => {
      result.current.confirmAccept();
    });

    await waitFor(() => expect(toast.warning).toHaveBeenCalled());
    expect(toast.warning.mock.calls[0]?.[0]).toContain("1 failed");
    expect(result.current.selection.ids).toEqual(["active-1"]);
  });

  it("never resumes a task that is not cancelled any more", async () => {
    vi.spyOn(apiClient, "getTask").mockResolvedValue(task({ task_id: "cancelled-1", status: "failed" }));
    vi.spyOn(apiClient, "rerunTask").mockResolvedValue({ task: { task_id: "cancelled-1" } });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ task_id: "cancelled-1", status: "cancelled" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    await act(async () => {
      result.current.runBulkAction("resume");
    });

    await waitFor(() => expect(apiClient.getTask).toHaveBeenCalledWith("cancelled-1"));
    expect(apiClient.rerunTask).not.toHaveBeenCalled();
    await waitFor(() => expect(toast.warning).toHaveBeenCalled());
  });

  it("requires a confirmation with the selected count before a permanent delete", async () => {
    vi.spyOn(apiClient, "deleteTask").mockResolvedValue(undefined);
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "trashed-1", status: "failed", deleted_at: "2026-01-02T00:00:00Z" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    act(() => {
      result.current.setView("trash");
    });
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
    });
    act(() => {
      result.current.runBulkAction("delete");
    });
    expect(apiClient.deleteTask).not.toHaveBeenCalled();
    expect(result.current.confirm?.message).toContain("1");

    await act(async () => {
      result.current.confirmAccept();
    });
    await waitFor(() => expect(apiClient.deleteTask).toHaveBeenCalledWith("trashed-1"));
  });

  it("clears a narrowing status filter after a cancelled task is resumed", async () => {
    vi.spyOn(apiClient, "rerunTask").mockResolvedValue({ task: { task_id: "cancelled-1" } });
    vi.mocked(apiClient.listTasks).mockImplementation(async (params) => (
      params.status === "cancelled"
        ? page([task({ status: "cancelled" })])
        : page([task({ status: "queued" })])
    ));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    act(() => {
      result.current.setStatus("cancelled");
    });
    await waitFor(() => expect(result.current.items[0]?.status).toBe("cancelled"));

    await act(async () => {
      result.current.recoverTask(result.current.items[0]!);
    });
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: null }),
      expect.anything(),
    ));
  });

  it("cancels only the selected active ids", async () => {
    vi.spyOn(apiClient, "cancelTask").mockResolvedValue(undefined);
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([
      task({ task_id: "active-1", status: "running" }),
      task({ task_id: "active-2", status: "waiting" }),
      task({ task_id: "done-1", status: "succeeded" }),
    ]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(3));

    act(() => {
      result.current.toggleTaskSelected(result.current.items[0]!);
      result.current.toggleTaskSelected(result.current.items[1]!);
    });
    act(() => {
      result.current.runBulkAction("cancel");
    });
    await act(async () => {
      result.current.confirmAccept();
    });
    await waitFor(() => expect(apiClient.cancelTask).toHaveBeenCalledTimes(2));
    expect(apiClient.cancelTask).toHaveBeenCalledWith("active-1");
    expect(apiClient.cancelTask).toHaveBeenCalledWith("active-2");
  });

  it("offers no clear-history command and keeps the history endpoint unused", async () => {
    vi.spyOn(apiClient, "clearTaskHistory").mockResolvedValue({ deleted_count: 3 });
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.pagination).not.toBeNull());

    act(() => {
      result.current.setView("completed");
    });
    await waitFor(() => expect(apiClient.listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed" }),
      expect.anything(),
    ));

    expect(apiClient.clearTaskHistory).not.toHaveBeenCalled();
    expect(result.current.bulkActions).toEqual([]);
  });

  it("keeps expanded rows across a same-window reload and drops them when the window changes", async () => {
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.toggleExpanded("task-1");
    });
    expect(result.current.expanded["task-1"]).toBe(true);

    await act(async () => {
      result.current.refresh();
    });
    await waitFor(() => expect(result.current.expanded["task-1"]).toBe(true));

    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ task_id: "task-9" })]));
    await act(async () => {
      result.current.setPage(2);
    });
    await waitFor(() => expect(result.current.items[0]?.task_id).toBe("task-9"));
    await waitFor(() => expect(result.current.expanded["task-1"]).toBeUndefined());
  });

  it("ignores a second action while the same task is already acting", async () => {
    const gate = deferred<{ retried_items: number; task: { task_id: string } }>();
    vi.spyOn(apiClient, "retryTask").mockReturnValue(gate.promise);
    vi.mocked(apiClient.listTasks).mockResolvedValue(page([task({ status: "failed" })]));
    const { result } = renderQueue();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => {
      result.current.recoverTask(result.current.items[0]!);
    });
    act(() => {
      result.current.recoverTask(result.current.items[0]!);
    });
    await pg();
    expect(apiClient.retryTask).toHaveBeenCalledTimes(1);

    await act(async () => {
      gate.resolve({ retried_items: 1, task: { task_id: "failed-1" } });
    });
  });
});

describe("useQueue list deadline", () => {
  beforeEach(() => {
    installFakeEventSource();
    // Only wall-clock timers are faked: React's commit scheduler and React
    // Query's batching keep running, so a fired deadline is observable without
    // pretending the product has a longer grace period.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] });
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task()]));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  /** Runs queued wall-clock timers without a real wait. */
  async function advance(ms: number) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
    await pg();
    await pg();
  }

  /**
   * Lets React Query publish the state it already decided. The fake clock owns
   * wall-clock timers only, so this is the store notification reaching the
   * observer; a request stopped by the deadline was aborted by `advance`.
   */
  async function publish() {
    await advance(1);
    for (let i = 0; i < 4; i += 1) {
      await pg();
    }
  }

  it("stays pending just before the deadline and fails at 15s", async () => {
    const read = deferred<TaskPageResponse>();
    const list = vi.mocked(apiClient.listTasks).mockReturnValue(read.promise);
    const { result } = renderQueue();
    await pg();

    expect(result.current.loading).toBe(true);
    expect(result.current.error).toBeNull();
    expect(list).toHaveBeenCalledTimes(1);
    const requestSignal = list.mock.calls[0][1]?.signal as AbortSignal;
    expect(requestSignal.aborted).toBe(false);

    // The request began at this clock's zero; the deadline is exactly 15s.
    await advance(QUEUE_LIST_TIMEOUT_MS - 1);
    expect(result.current.loading).toBe(true);
    expect(result.current.error).toBeNull();
    expect(requestSignal.aborted).toBe(false);

    // At 15s the deadline fires: the transport is aborted and the read fails.
    await advance(1);
    expect(requestSignal.aborted).toBe(true);
    await publish();

    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeInstanceOf(Error);
    expect(String(result.current.error)).toContain("timed out");
    expect(result.current.items).toEqual([]);
  });

  it("accepts a manual refresh after the timeout and loads rows", async () => {
    const read = deferred<TaskPageResponse>();
    vi.mocked(apiClient.listTasks)
      .mockReturnValueOnce(read.promise)
      .mockResolvedValue(page([task()]));
    const { result } = renderQueue();
    await pg();

    await advance(QUEUE_LIST_TIMEOUT_MS);
    await publish();
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeInstanceOf(Error);

    await act(async () => {
      result.current.refresh();
    });
    await publish();

    expect(result.current.items).toHaveLength(1);
    expect(result.current.error).toBeNull();
    expect(vi.mocked(apiClient.listTasks)).toHaveBeenCalledTimes(2);
  });

  it("retains the rows that were already loaded when a refresh times out", async () => {
    vi.mocked(apiClient.listTasks)
      .mockResolvedValueOnce(page([task()]))
      .mockReturnValueOnce(new Promise<TaskPageResponse>(() => undefined));
    const { result } = renderQueue();
    await publish();
    expect(result.current.items).toHaveLength(1);

    await act(async () => {
      result.current.refresh();
    });
    await pg();
    await advance(QUEUE_LIST_TIMEOUT_MS);
    await publish();

    expect(result.current.error).toBeInstanceOf(Error);
    expect(result.current.items).toHaveLength(1);
    expect(result.current.items[0]?.task_id).toBe("task-1");
  });

  it("admits one queued refresh after the timeout, not several", async () => {
    const read = deferred<TaskPageResponse>();
    vi.mocked(apiClient.listTasks)
      .mockReturnValueOnce(read.promise)
      .mockResolvedValue(page([task()]));
    const { result } = renderQueue();
    await pg();

    act(() => {
      result.current.refresh();
      result.current.refresh();
      result.current.refresh();
    });
    await pg();
    expect(vi.mocked(apiClient.listTasks)).toHaveBeenCalledTimes(1);

    await advance(QUEUE_LIST_TIMEOUT_MS);
    expect(vi.mocked(apiClient.listTasks)).toHaveBeenCalledTimes(1);
    await publish();
    await publish();

    expect(result.current.items).toHaveLength(1);
    // The hung read plus exactly one trailing authoritative read.
    expect(vi.mocked(apiClient.listTasks)).toHaveBeenCalledTimes(2);
  });

  /** The id of the deadline timer the hook registered, plus a clear spy. */
  function trackDeadlineTimer() {
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");
    const clearTimeoutSpy = vi.spyOn(globalThis, "clearTimeout");
    const findId = () => {
      const index = setTimeoutSpy.mock.calls.findIndex(([, delay]) => delay === QUEUE_LIST_TIMEOUT_MS);
      return index < 0 ? undefined : setTimeoutSpy.mock.results[index]?.value;
    };
    const cleared = (id: unknown) => clearTimeoutSpy.mock.calls.some(([timerId]) => timerId === id);
    return { findId, cleared };
  }

  it("cleans the deadline when the query is cancelled by unmount", async () => {
    const { findId, cleared } = trackDeadlineTimer();
    vi.mocked(apiClient.listTasks).mockReturnValue(new Promise<TaskPageResponse>(() => undefined));
    const { unmount, result } = renderQueue();
    await pg();
    const requestSignal = vi.mocked(apiClient.listTasks).mock.calls[0][1]?.signal as AbortSignal;
    const deadlineId = findId();
    expect(deadlineId).toBeDefined();
    expect(cleared(deadlineId)).toBe(false);

    await act(async () => {
      unmount();
    });
    expect(requestSignal.aborted).toBe(true);
    expect(cleared(deadlineId)).toBe(true);

    // No hidden retry or late failure after the component is gone.
    await advance(QUEUE_LIST_TIMEOUT_MS * 2);
    expect(result.current.error).toBeNull();
    expect(vi.mocked(apiClient.listTasks)).toHaveBeenCalledTimes(1);
  });

  it("clears the deadline timer after a successful read", async () => {
    const { findId, cleared } = trackDeadlineTimer();
    const { result } = renderQueue();
    await pg();
    const deadlineId = findId();
    expect(deadlineId).toBeDefined();

    await publish();
    expect(result.current.items).toHaveLength(1);
    expect(cleared(deadlineId)).toBe(true);

    // Long past the original deadline nothing aborts, retries or refetches.
    await advance(QUEUE_LIST_TIMEOUT_MS * 2);
    expect(result.current.items).toHaveLength(1);
    expect(result.current.error).toBeNull();
    expect(vi.mocked(apiClient.listTasks)).toHaveBeenCalledTimes(1);
  });

  it("never lands an ignored-abort response after the deadline", async () => {
    const late = deferred<TaskPageResponse>();
    vi.mocked(apiClient.listTasks)
      .mockReturnValueOnce(late.promise)
      .mockResolvedValue(page([task({ task_id: "task-fresh" })]));
    const { result } = renderQueue();
    await pg();

    await advance(QUEUE_LIST_TIMEOUT_MS);
    await publish();
    expect(result.current.error).toBeInstanceOf(Error);

    // The transport ignored the abort and answers 1s after the deadline.
    await act(async () => {
      late.resolve(page([task({ task_id: "task-late" })]));
    });
    await publish();

    expect(result.current.items).toEqual([]);
    expect(result.current.error).toBeInstanceOf(Error);

    // A later read is not confused by the abandoned response: it lands normally.
    await act(async () => {
      result.current.refresh();
    });
    await publish();
    expect(result.current.items.map((item) => item.task_id)).toEqual(["task-fresh"]);
    expect(result.current.error).toBeNull();
  });
});

/** Flushes pending microtasks without advancing fake timers. */
async function pg() {
  await Promise.resolve();
  await Promise.resolve();
}
