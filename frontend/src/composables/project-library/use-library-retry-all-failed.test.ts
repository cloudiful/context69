import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import {
  collectRecursiveFailedFileIds,
  useLibraryRetryAllFailed,
} from "./use-library-retry-all-failed";

const confirmMocks = vi.hoisted(() => ({ require: vi.fn() }));
vi.mock("../use-app-confirm", () => ({
  useAppConfirm: () => ({ require: confirmMocks.require }),
}));

const getGroupLibraryResources = vi.spyOn(apiClient, "getGroupLibraryResources");
const getGroupLibraryFile = vi.spyOn(apiClient, "getGroupLibraryFile");
const submitTask = vi.spyOn(apiClient, "submitTask");
const getTask = vi.spyOn(apiClient, "getTask");

interface ConfirmOptions {
  header: string;
  message: string;
  accept?: () => void | Promise<void>;
}

function failedPage(ids: string[], total: number, page = 1) {
  return {
    items: ids.map((id) => ({
      child_folder_count: 0,
      created_at: "2026-07-11T10:00:00Z",
      file_count: 0,
      group_key: "alpha",
      group_path: "stock/alpha",
      id,
      ingest_status: "failed",
      is_source_folder: false,
      is_source_records_folder: false,
      kind: "file",
      media_type: "text/plain",
      name: `${id}.txt`,
      parent_folder_id: null,
      processing_count: 0,
      size_bytes: 10,
      updated_at: "2026-07-11T12:00:00Z",
      visibility: "private",
    })),
    pagination: { page, page_size: 100, total, total_pages: Math.ceil(total / 100) },
  } as never;
}

function setup(options: { folderId?: string | null } = {}) {
  let state!: ReturnType<typeof useLibraryRetryAllFailed>;
  const refresh = vi.fn().mockResolvedValue(undefined);
  const unavailable = ref<string[]>([]);
  mount(defineComponent({
    setup() {
      state = useLibraryRetryAllFailed({
        groupPath: ref("stock/alpha"),
        folderId: ref(options.folderId ?? null),
        refresh,
        isSourceUnavailable: (id: string) => unavailable.value.includes(id),
        observeSourceAvailability: (detail) => {
          if (!detail.source_available) unavailable.value = [...unavailable.value, detail.file_id];
        },
        t: ((key: string, params?: Record<string, unknown>) => {
          if (key === "library.retryAllConfirm") return `retry ${params?.count}`;
          return key;
        }) as never,
      });
      return {};
    },
    template: "<div />",
  }), { global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] } });
  return { state, refresh, unavailable };
}

describe("useLibraryRetryAllFailed", () => {
  beforeEach(() => {
    confirmMocks.require.mockReset();
    getGroupLibraryResources.mockReset();
    getGroupLibraryFile.mockReset();
    submitTask.mockReset();
    getTask.mockReset();
    getTask.mockResolvedValue({ task_id: "task-id", status: "succeeded" } as never);
  });

  it("enables retry-all only when failed files exist", async () => {
    getGroupLibraryResources.mockResolvedValue(failedPage([], 0));
    const { state } = setup();
    await state.loadFailedCount();
    expect(state.retryAllFailedCount.value).toBe(0);

    state.retryAllFailed();
    expect(confirmMocks.require).not.toHaveBeenCalled();

    getGroupLibraryResources.mockResolvedValue(failedPage(["f1"], 1));
    await state.loadFailedCount();
    expect(state.retryAllFailedCount.value).toBe(1);

    state.retryAllFailed();
    expect(confirmMocks.require).toHaveBeenCalledOnce();
  });

  it("collects failed file ids across pages with recursive scope", async () => {
    getGroupLibraryResources
      .mockResolvedValueOnce(failedPage(["f1", "f2"], 3, 1))
      .mockResolvedValueOnce(failedPage(["f3"], 3, 2));
    const ids = await collectRecursiveFailedFileIds("stock/alpha", "folder-1");
    expect(ids).toEqual(["f1", "f2", "f3"]);
    expect(getGroupLibraryResources).toHaveBeenCalledTimes(2);
    expect(getGroupLibraryResources).toHaveBeenCalledWith("stock/alpha", expect.objectContaining({
      folderId: "folder-1",
      recursive: true,
      pageSize: 100,
      status: "failed",
    }));
  });

  it("submits collected ids once and skips files with missing sources", async () => {
    getGroupLibraryResources.mockResolvedValue(failedPage(["f1", "f2", "f3"], 3, 1));
    getGroupLibraryFile.mockImplementation(async (_group: string, fileId: string) => ({
      file_id: fileId,
      source_available: fileId !== "f2",
    }) as never);
    submitTask.mockResolvedValue({ task_id: "task-id", item_ids: ["i1"] } as never);
    const { state, refresh, unavailable } = setup();
    let captured: ConfirmOptions | undefined;
    confirmMocks.require.mockImplementation((options: ConfirmOptions) => { captured = options; });
    getGroupLibraryResources.mockResolvedValueOnce(failedPage(["f1"], 1, 1));
    await state.loadFailedCount();
    state.retryAllFailed();
    expect(captured?.message).toContain("1");

    getGroupLibraryResources.mockResolvedValue(failedPage(["f1", "f2", "f3"], 3, 1));
    await captured!.accept?.();
    await flushPromises();

    expect(submitTask).toHaveBeenCalledTimes(1);
    expect(submitTask).toHaveBeenCalledWith({
      kind: "retry_file_batch",
      group_path: "stock/alpha",
      items: [{ file_id: "f1" }, { file_id: "f3" }],
    });
    expect(unavailable.value).toEqual(["f2"]);
    expect(refresh).toHaveBeenCalled();
    expect(state.retryAllBusy.value).toBe(false);
  });

  it("disables the button while a retry-all batch is in flight", async () => {
    getGroupLibraryResources.mockResolvedValue(failedPage(["f1"], 1, 1));
    getGroupLibraryFile.mockResolvedValue({ file_id: "f1", source_available: true } as never);
    submitTask.mockResolvedValue({ task_id: "task-id", item_ids: ["i1"] } as never);
    const { state } = setup();
    await state.loadFailedCount();
    const pending = state.retryAllFailedConfirmed();
    expect(state.retryAllBusy.value).toBe(true);
    await pending;
    await flushPromises();
    expect(state.retryAllBusy.value).toBe(false);
  });
});
