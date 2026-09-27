import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskPageResponse, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { useSettingsVectorRebuild } from "./use-settings-vector-rebuild";

const mocks = vi.hoisted(() => ({ addToast: vi.fn() }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.addToast }) }));

const confirmMocks = vi.hoisted(() => ({ require: vi.fn() }));
vi.mock("./use-app-confirm", () => ({
  useAppConfirm: () => ({ require: confirmMocks.require }),
}));

function task(overrides: Partial<TaskResponse> = {}): TaskResponse {
  return {
    task_id: "00000000-0000-0000-0000-000000000001",
    kind: "vector_rebuild",
    origin: "manual",
    status: "running",
    group_path: null,
    source_key: null,
    stage: null,
    waiting_reason: null,
    dependency_key: null,
    progress: { total: 0, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-08-02T00:00:00Z",
    started_at: "2026-08-02T00:00:00Z",
    finished_at: null,
    updated_at: "2026-08-02T00:00:00Z",
    ...overrides,
  };
}

function page(items: TaskResponse[]): TaskPageResponse {
  return {
    items,
    pagination: {
      page: 1,
      page_size: 1,
      total: items.length,
      total_pages: items.length === 0 ? 0 : 1,
    },
  };
}

function mountVectorRebuild() {
  let state!: ReturnType<typeof useSettingsVectorRebuild>;
  const wrapper = mount(defineComponent({
    setup() {
      state = useSettingsVectorRebuild();
      return {};
    },
    template: "<div />",
  }), {
    global: { plugins: [createTestI18n("en")] },
  });

  return { state, wrapper };
}

describe("useSettingsVectorRebuild", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.addToast.mockReset();
    confirmMocks.require.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("loads the processing task with the locked request shape and adopts the first item", async () => {
    const running = task({ status: "running" });
    const listTasks = vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([running]));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    await state.loadVectorRebuildTask();

    expect(listTasks).toHaveBeenCalledWith({
      page: 1,
      pageSize: 1,
      view: "processing",
      kind: "vector_rebuild",
      status: null,
      stage: null,
      waitingReason: null,
      dependencyKey: null,
    });
    expect(state.vectorRebuildStatus.value).toEqual(running);
    state.clearVectorRebuildPoll();
    wrapper.unmount();
  });

  it("clears the status when the processing view returns no task", async () => {
    vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([]));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    state.vectorRebuildStatus.value = task();
    await state.loadVectorRebuildTask();

    expect(state.vectorRebuildStatus.value).toBeNull();
    wrapper.unmount();
  });

  it("reloads active rebuild tasks on the poll interval and stops once terminal", async () => {
    const listTasks = vi.spyOn(apiClient, "listTasks")
      .mockResolvedValueOnce(page([task({ status: "queued" })]))
      .mockResolvedValueOnce(page([task({ status: "succeeded" })]));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    await state.loadVectorRebuildTask();
    expect(listTasks).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1500);
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.vectorRebuildStatus.value?.status).toBe("succeeded");

    await vi.advanceTimersByTimeAsync(5000);
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("does not poll for absent or terminal tasks", async () => {
    const listTasks = vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ status: "succeeded" })]));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    await state.loadVectorRebuildTask();
    await vi.advanceTimersByTimeAsync(5000);
    expect(listTasks).toHaveBeenCalledTimes(1);

    listTasks.mockResolvedValue(page([]));
    await state.loadVectorRebuildTask();
    await vi.advanceTimersByTimeAsync(5000);
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.vectorRebuildStatus.value).toBeNull();
    wrapper.unmount();
  });

  it("surfaces a poll failure without dropping the last known status", async () => {
    vi.spyOn(apiClient, "listTasks")
      .mockResolvedValueOnce(page([task({ status: "running" })]))
      .mockRejectedValueOnce(new Error("poll down"));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    await state.loadVectorRebuildTask();
    await vi.advanceTimersByTimeAsync(1500);

    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      color: "error",
      description: "poll down",
    }));
    expect(state.vectorRebuildStatus.value?.status).toBe("running");
    wrapper.unmount();
  });

  it("stops the scheduled poll when the caller clears it", async () => {
    const listTasks = vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ status: "running" })]));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    await state.loadVectorRebuildTask();
    state.clearVectorRebuildPoll();
    await vi.advanceTimersByTimeAsync(5000);

    expect(listTasks).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("submits a rebuild, adopts the fetched task, toasts, and schedules the poll", async () => {
    const started = task({ task_id: "task-1", status: "running" });
    const submitVectorIndexRebuild = vi.spyOn(apiClient, "submitVectorIndexRebuild")
      .mockResolvedValue({ task_id: "task-1", item_ids: [] } as never);
    const getTask = vi.spyOn(apiClient, "getTask").mockResolvedValue(started);
    const listTasks = vi.spyOn(apiClient, "listTasks").mockResolvedValue(page([task({ task_id: "task-1", status: "succeeded" })]));
    vi.useFakeTimers();

    const { state, wrapper } = mountVectorRebuild();
    await state.startVectorIndexRebuild();

    expect(submitVectorIndexRebuild).toHaveBeenCalledWith();
    expect(getTask).toHaveBeenCalledWith("task-1");
    expect(state.vectorRebuildStatus.value).toEqual(started);
    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({ color: "info" }));

    await vi.advanceTimersByTimeAsync(1500);
    expect(listTasks).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("reports a failed start without fetching a task or scheduling a poll", async () => {
    const submitVectorIndexRebuild = vi.spyOn(apiClient, "submitVectorIndexRebuild")
      .mockRejectedValue(new Error("submit failed"));
    const getTask = vi.spyOn(apiClient, "getTask").mockResolvedValue(task());

    const { state, wrapper } = mountVectorRebuild();
    await state.startVectorIndexRebuild();

    expect(submitVectorIndexRebuild).toHaveBeenCalledTimes(1);
    expect(getTask).not.toHaveBeenCalled();
    expect(state.vectorRebuildStatus.value).toBeNull();
    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      color: "error",
      description: "submit failed",
    }));
    wrapper.unmount();
  });

  it("confirms before starting the rebuild", async () => {
    const submitVectorIndexRebuild = vi.spyOn(apiClient, "submitVectorIndexRebuild")
      .mockResolvedValue({ task_id: "task-1", item_ids: [] } as never);
    vi.spyOn(apiClient, "getTask").mockResolvedValue(task({ task_id: "task-1", status: "succeeded" }));

    const { state, wrapper } = mountVectorRebuild();
    state.confirmVectorIndexRebuild();

    expect(confirmMocks.require).toHaveBeenCalledWith(expect.objectContaining({
      accept: expect.any(Function),
    }));

    const options = confirmMocks.require.mock.calls[0][0] as { accept: () => void };
    options.accept();
    await flushPromises();

    expect(submitVectorIndexRebuild).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });
});
