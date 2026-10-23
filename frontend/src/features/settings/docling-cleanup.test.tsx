import { act, renderHook, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { i18n } from "@/lib/i18n";
import { apiClient } from "@/services/api";
import { ApiError } from "@/services/api/api-core";
import type { DoclingCleanupPageResponse, DoclingCleanupTask } from "@/services/api";

import { useDoclingCleanup } from "./use-docling-cleanup";

function row(overrides: Partial<DoclingCleanupTask> = {}): DoclingCleanupTask {
  return {
    id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    task_id: "11111111-1111-4111-8111-111111111111",
    remote_task_id: "cleanup-1",
    status: "pending",
    attempt_count: 0,
    cleanup_deadline_at: "2026-10-20T09:00:00Z",
    next_attempt_at: "2026-10-20T08:05:00Z",
    outcome: null,
    reason_code: "unspecified",
    created_at: "2026-10-20T08:00:00Z",
    updated_at: "2026-10-20T08:01:00Z",
    finished_at: null,
    ...overrides,
  };
}

function response(
  items: DoclingCleanupTask[],
  pagination: DoclingCleanupPageResponse["pagination"] = {
    page: 1,
    page_size: 20,
    total: items.length,
    total_pages: 1,
  },
): DoclingCleanupPageResponse {
  return { items, pagination };
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

function renderCleanup() {
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
  );
  return renderHook(() => useDoclingCleanup(), { wrapper });
}

describe("useDoclingCleanup", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    vi.spyOn(apiClient, "listDoclingCleanup").mockResolvedValue(response([row()]));
  });

  it("reads the first page with the default page size and no filters", async () => {
    const { result } = renderCleanup();

    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenCalledWith({
      page: 1,
      pageSize: 20,
      status: null,
      outcome: null,
      remoteTaskId: "",
    }));
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    expect(result.current.pagination).toMatchObject({ page: 1, page_size: 20, total: 1 });
  });

  it("resets to page one when the page size changes", async () => {
    vi.mocked(apiClient.listDoclingCleanup).mockResolvedValue(response([row()], {
      page: 2,
      page_size: 20,
      total: 60,
      total_pages: 3,
    }));
    const { result } = renderCleanup();
    await waitFor(() => expect(result.current.pagination).not.toBeNull());

    act(() => result.current.setPage(2));
    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2, pageSize: 20 }),
    ));

    act(() => result.current.setPageSize(50));
    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 1, pageSize: 50 }),
    ));
  });

  it("sends exact filters and resets to page one when a filter changes", async () => {
    const { result } = renderCleanup();
    await waitFor(() => expect(result.current.pagination).not.toBeNull());

    act(() => result.current.setPage(2));
    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2 }),
    ));

    act(() => {
      result.current.setStatus("done");
      result.current.setOutcome("success");
      result.current.setRemoteTaskId(" cleanup-2 ");
    });

    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith({
      page: 1,
      pageSize: 20,
      status: "done",
      outcome: "success",
      remoteTaskId: " cleanup-2 ",
    }));
  });

  it("ignores a stale response that resolves after a newer query", async () => {
    const first = deferred<DoclingCleanupPageResponse>();
    const second = deferred<DoclingCleanupPageResponse>();
    vi.mocked(apiClient.listDoclingCleanup)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const { result } = renderCleanup();
    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenCalledTimes(1));

    act(() => result.current.setStatus("failed"));
    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenCalledTimes(2));

    await act(async () => {
      second.resolve(response([row({ id: "new", remote_task_id: "new", status: "failed" })]));
      await second.promise;
    });
    await waitFor(() => expect(result.current.items[0]?.remote_task_id).toBe("new"));

    await act(async () => {
      first.resolve(response([row({ id: "stale", remote_task_id: "stale" })]));
      await first.promise;
    });

    expect(result.current.items[0]?.remote_task_id).toBe("new");
  });

  it("does not restore an old query when a wake completes after filters change", async () => {
    const wake = deferred<{ woken: number }>();
    vi.spyOn(apiClient, "wakeDoclingCleanup").mockReturnValue(wake.promise);
    vi.mocked(apiClient.listDoclingCleanup).mockImplementation(async (query) =>
      response([row({ remote_task_id: query?.status === "failed" ? "failed-row" : "unfiltered-row" })], {
        page: query.page,
        page_size: query.pageSize,
        total: query.status === "failed" ? 1 : 120,
        total_pages: query.status === "failed" ? 1 : 6,
      }),
    );
    const { result } = renderCleanup();
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    let waking!: Promise<void>;
    act(() => { waking = result.current.wakeCleanup(); });
    act(() => result.current.setStatus("failed"));
    await waitFor(() => expect(result.current.items[0]?.remote_task_id).toBe("failed-row"));
    await act(async () => {
      wake.resolve({ woken: 1 });
      await waking;
    });
    expect(result.current.status).toBe("failed");
    expect(result.current.items[0]?.remote_task_id).toBe("failed-row");
    expect(result.current.pagination).toEqual({ page: 1, page_size: 20, total: 1, total_pages: 1 });
    expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 1, pageSize: 20, status: "failed" }),
    );
  });

  it("reports a load failure instead of an empty success state", async () => {
    vi.mocked(apiClient.listDoclingCleanup).mockRejectedValue(
      new ApiError("cleanup unavailable", 500),
    );

    const { result } = renderCleanup();

    await waitFor(() => expect(result.current.error).toContain("cleanup unavailable"));
    expect(result.current.errorStatus).toBe(500);
    expect(result.current.loadedOnce).toBe(false);
  });

  it("wakes every eligible job and refreshes the current selection", async () => {
    vi.spyOn(apiClient, "wakeDoclingCleanup").mockResolvedValue({ woken: 4 });
    const { result } = renderCleanup();
    await waitFor(() => expect(result.current.items).toHaveLength(1));

    act(() => result.current.setStatus("pending"));
    await waitFor(() => expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "pending" }),
    ));

    await act(async () => {
      await result.current.wakeCleanup();
    });

    expect(apiClient.wakeDoclingCleanup).toHaveBeenCalledOnce();
    expect(result.current.woken).toBe(4);
    expect(apiClient.listDoclingCleanup).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "pending", page: 1 }),
    );
  });
});
