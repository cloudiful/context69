import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { mount } from "@vue/test-utils";

import { apiClient } from "../services/api";
import { ApiError } from "../services/api/api-core";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { useDoclingRemoteTasks } from "./use-docling-remote-tasks";

function mountComposable() {
  let captured!: ReturnType<typeof useDoclingRemoteTasks>;
  const host = defineComponent({
    setup() {
      captured = useDoclingRemoteTasks();
      return () => h("div");
    },
  });
  mount(host, {
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
  return captured;
}

function remotePage(items: { task_id: string; task_status: string }[], has_more: boolean) {
  return {
    items: items.map((item, index) => ({
      task_id: item.task_id,
      task_type: "convert",
      task_status: item.task_status,
      created_at: `2026-10-01T0${index}:00:00Z`,
    })),
    has_more,
  };
}

beforeEach(() => {
  vi.restoreAllMocks();
});

describe("useDoclingRemoteTasks", () => {
  it("keeps owned cleanup visible and actionable when remote listing fails", async () => {
    const cleanup = [{ id: "c", task_id: "t", remote_task_id: "r", status: "failed", outcome: null, attempt_count: 2, next_attempt_at: "2026-10-01T00:00:00Z", cleanup_deadline_at: "2026-10-02T00:00:00Z" }];
    vi.spyOn(apiClient, "listDoclingRemoteTasks").mockResolvedValue({ items: [], has_more: false, cleanup, remote_error: "Remote activity unavailable" });
    const wake = vi.spyOn(apiClient, "wakeDoclingCleanup").mockResolvedValue({ woken: 0 });
    const state = mountComposable();
    await state.loadDoclingRemoteTasks();
    expect(state.doclingCleanupItems.value).toEqual(cleanup);
    expect(state.doclingRemoteError.value).toBe("Remote activity unavailable");
    expect(state.doclingRemoteErrorStatus.value).toBeNull();
    await state.cleanStoppedDoclingTasks();
    expect(wake).toHaveBeenCalledOnce();
    expect(state.doclingCleanupItems.value).toEqual(cleanup);
  });
  it("wakes only owned cleanup and refreshes its state", async () => {
    const wake = vi.spyOn(apiClient, "wakeDoclingCleanup").mockResolvedValue({ woken: 1 });
    vi.spyOn(apiClient, "listDoclingRemoteTasks").mockResolvedValue({
      ...remotePage([], false),
      cleanup: [{ id: "c", task_id: "t", remote_task_id: "r", status: "unsupported", outcome: null, attempt_count: 1, next_attempt_at: "2026-10-01T00:00:00Z", cleanup_deadline_at: "2026-10-02T00:00:00Z" }],
    });
    const state = mountComposable();
    await state.cleanStoppedDoclingTasks();
    expect(wake).toHaveBeenCalledWith();
    expect(state.doclingCleanupItems.value[0]?.status).toBe("unsupported");
    expect(state.doclingCleaning.value).toBe(false);
  });

  it("drops cleanup refresh after reset and surfaces denial", async () => {
    let finish!: (value: { woken: number }) => void;
    const wake = vi.spyOn(apiClient, "wakeDoclingCleanup").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const list = vi.spyOn(apiClient, "listDoclingRemoteTasks");
    const state = mountComposable();
    const pending = state.cleanStoppedDoclingTasks();
    state.resetDoclingRemoteTasks();
    finish({ woken: 1 });
    await pending;
    expect(list).not.toHaveBeenCalled();
    expect(state.doclingCleanupItems.value).toEqual([]);
    wake.mockRejectedValue(new ApiError("denied", 403));
    await state.cleanStoppedDoclingTasks();
    expect(state.doclingRemoteErrorStatus.value).toBe(403);
  });
  it("loads one bounded page with limit 20 and offset 0", async () => {
    const list = vi
      .spyOn(apiClient, "listDoclingRemoteTasks")
      .mockResolvedValue(remotePage([{ task_id: "task-1", task_status: "started" }], false) as never);
    const state = mountComposable();

    await state.loadDoclingRemoteTasks();

    expect(list).toHaveBeenCalledWith({ limit: 20, offset: 0 });
    expect(state.doclingRemoteItems.value).toHaveLength(1);
    expect(state.doclingRemoteHasMore.value).toBe(false);
    expect(state.doclingRemoteLoadedOnce.value).toBe(true);
    expect(state.doclingRemoteError.value).toBeNull();
  });

  it("pages forward on has_more and back to the first page", async () => {
    const list = vi.spyOn(apiClient, "listDoclingRemoteTasks").mockImplementation(async (params: {
      limit: number;
      offset: number;
    }) => {
      if (params.offset === 0) {
        return remotePage([{ task_id: "task-1", task_status: "started" }], true) as never;
      }
      return remotePage([{ task_id: "task-2", task_status: "unknown_future" }], false) as never;
    });
    const state = mountComposable();

    await state.loadDoclingRemoteTasks();
    await state.nextDoclingRemotePage();

    expect(list).toHaveBeenLastCalledWith({ limit: 20, offset: 20 });
    expect(state.doclingRemoteItems.value[0]?.task_id).toBe("task-2");
    // Unknown statuses stay text for the UI to render verbatim.
    expect(state.doclingRemoteItems.value[0]?.task_status).toBe("unknown_future");

    await state.prevDoclingRemotePage();

    expect(list).toHaveBeenLastCalledWith({ limit: 20, offset: 0 });
    expect(state.doclingRemoteItems.value[0]?.task_id).toBe("task-1");
  });

  it("does not page past the first page or beyond has_more", async () => {
    const list = vi
      .spyOn(apiClient, "listDoclingRemoteTasks")
      .mockResolvedValue(remotePage([{ task_id: "task-1", task_status: "started" }], false) as never);
    const state = mountComposable();

    await state.loadDoclingRemoteTasks();
    await state.nextDoclingRemotePage();
    await state.prevDoclingRemotePage();

    expect(list).toHaveBeenCalledTimes(1);
    expect(state.doclingRemoteOffset.value).toBe(0);
  });

  it("keeps the last good page when a refresh fails", async () => {
    const list = vi.spyOn(apiClient, "listDoclingRemoteTasks");
    list.mockResolvedValueOnce(
      remotePage([{ task_id: "task-1", task_status: "started" }], false) as never,
    );
    list.mockRejectedValueOnce(new ApiError("remote unavailable", 503));
    const state = mountComposable();

    await state.loadDoclingRemoteTasks();
    await state.refreshDoclingRemoteTasks();

    expect(state.doclingRemoteItems.value).toHaveLength(1);
    expect(state.doclingRemoteError.value).toBe("remote unavailable");
    expect(state.doclingRemoteErrorStatus.value).toBe(503);
  });

  it("surfaces admin denial with its 403 status", async () => {
    vi.spyOn(apiClient, "listDoclingRemoteTasks").mockRejectedValueOnce(
      new ApiError("admin access required", 403),
    );
    const state = mountComposable();

    await state.loadDoclingRemoteTasks();

    expect(state.doclingRemoteItems.value).toHaveLength(0);
    expect(state.doclingRemoteErrorStatus.value).toBe(403);
    expect(state.doclingRemoteError.value).toBe("admin access required");
  });

  it("drops a stale response when offset changes mid-flight", async () => {
    let resolveFirst: ((value: unknown) => void) | undefined;
    const first = new Promise((resolve) => {
      resolveFirst = resolve as (value: unknown) => void;
    });
    const list = vi.spyOn(apiClient, "listDoclingRemoteTasks");
    list.mockImplementationOnce(() => first as never);
    list.mockResolvedValueOnce(
      remotePage([{ task_id: "task-2", task_status: "started" }], false) as never,
    );
    const state = mountComposable();

    const pending = state.loadDoclingRemoteTasks();
    state.doclingRemoteOffset.value = 20;
    const second = state.loadDoclingRemoteTasks();
    resolveFirst!(remotePage([{ task_id: "stale", task_status: "started" }], true));
    await pending;
    await second;

    expect(state.doclingRemoteItems.value[0]?.task_id).toBe("task-2");
    expect(state.doclingRemoteHasMore.value).toBe(false);
  });

  it("resets pagination and cached state", async () => {
    vi.spyOn(apiClient, "listDoclingRemoteTasks").mockResolvedValue(
      remotePage([{ task_id: "task-1", task_status: "started" }], true) as never,
    );
    const state = mountComposable();

    await state.loadDoclingRemoteTasks();
    state.resetDoclingRemoteTasks();

    expect(state.doclingRemoteOffset.value).toBe(0);
    expect(state.doclingRemoteItems.value).toHaveLength(0);
    expect(state.doclingRemoteLoadedOnce.value).toBe(false);
    expect(state.doclingRemoteError.value).toBeNull();
  });

  it("records the woken count from cleanup, including zero", async () => {
    vi.spyOn(apiClient, "wakeDoclingCleanup").mockResolvedValue({ woken: 0 });
    vi.spyOn(apiClient, "listDoclingRemoteTasks").mockResolvedValue({
      ...remotePage([], false),
      cleanup: [],
    });
    const state = mountComposable();

    expect(state.doclingCleanupWoken.value).toBeNull();
    await state.cleanStoppedDoclingTasks();
    expect(state.doclingCleanupWoken.value).toBe(0);
  });

  it("records a nonzero woken count and clears it on reset", async () => {
    vi.spyOn(apiClient, "wakeDoclingCleanup").mockResolvedValue({ woken: 3 });
    vi.spyOn(apiClient, "listDoclingRemoteTasks").mockResolvedValue({
      ...remotePage([], false),
      cleanup: [],
    });
    const state = mountComposable();

    await state.cleanStoppedDoclingTasks();
    expect(state.doclingCleanupWoken.value).toBe(3);
    state.resetDoclingRemoteTasks();
    expect(state.doclingCleanupWoken.value).toBeNull();
  });
});
