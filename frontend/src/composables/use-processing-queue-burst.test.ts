import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, watch } from "vue";
import { useI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse, type TaskPageResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { useProcessingQueue } from "./use-processing-queue";

const mocks = vi.hoisted(() => ({ addToast: vi.fn(), require: vi.fn() }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.addToast }) }));
vi.mock("./use-app-confirm", () => ({ useAppConfirm: () => ({ require: mocks.require }) }));

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const rerunTask = vi.spyOn(apiClient, "rerunTask");

function task(index: number, status: TaskResponse["status"] = "running"): TaskResponse {
  return {
    task_id: `task-${String(index).padStart(3, "0")}`,
    kind: index % 2 === 0 ? "file_batch" : "url_batch",
    origin: "manual",
    status,
    group_path: "research",
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    progress: { total: 2, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-07-20T00:00:00Z",
    updated_at: "2026-07-20T00:01:00Z",
  };
}

function pageFor(tasks: TaskResponse[], params: Parameters<typeof apiClient.listTasks>[0]): TaskPageResponse {
  const filtered = tasks.filter((item) => {
    if (params.kind && item.kind !== params.kind) return false;
    if (params.status && item.status !== params.status) return false;
    if (params.view === "completed" && (item.deleted_at || item.status !== "succeeded")) return false;
    if (params.view === "trash" && !item.deleted_at) return false;
    if (params.view === "processing" && (item.deleted_at || item.status === "succeeded")) return false;
    return true;
  });
  const start = (params.page - 1) * params.pageSize;
  return {
    items: filtered.slice(start, start + params.pageSize).map((item) => ({ ...item, progress: { ...item.progress } })),
    pagination: {
      page: params.page,
      page_size: params.pageSize,
      total: filtered.length,
      total_pages: Math.ceil(filtered.length / params.pageSize),
    },
  };
}

function mountQueue(tasks: TaskResponse[]) {
  listTasks.mockImplementation((params) => Promise.resolve(pageFor(tasks, params)) as never);
  let state!: ReturnType<typeof useProcessingQueue>;
  const wrapper = mount(defineComponent({
    setup() {
      const { t } = useI18n();
      state = useProcessingQueue({ t });
      return {};
    },
    template: "<div />",
  }), { global: { plugins: [createTestI18n()] } });
  return { state, wrapper };
}

async function loadHundredTasks(tasks: TaskResponse[]) {
  installFakeEventSource();
  const { state, wrapper } = mountQueue(tasks);
  await flushPromises();
  state.changePageSize(100);
  await flushPromises();
  state.startLiveUpdates();
  const stream = takeEventSource();
  stream.emit("snapshot", { tasks: state.items.value });
  await flushPromises();
  expect(state.items.value).toHaveLength(100);
  return { state, wrapper, stream };
}

beforeEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  mocks.addToast.mockReset();
  mocks.require.mockReset();
  listTasks.mockReset();
  retryTask.mockReset();
  rerunTask.mockReset();
});
afterEach(() => vi.useRealTimers());

describe("processing queue burst behavior", () => {
  it("commits a 100-task frame burst and duplicate latest states once without list reloads", async () => {
    const tasks = Array.from({ length: 100 }, (_, index) => task(index));
    const { state, wrapper, stream } = await loadHundredTasks(tasks);
    const baselineLoads = listTasks.mock.calls.length;
    const originalItems = state.items.value;
    let commits = 0;
    const stopWatching = watch(state.items, () => { commits += 1; }, { flush: "sync" });
    vi.useFakeTimers();

    for (const current of originalItems) {
      stream.emit("update", {
        task: {
          ...current,
          stage: "indexing",
          progress: { ...current.progress, succeeded: 1, running: 0 },
        },
      });
    }
    const latest = { ...originalItems[0]!, stage: "storage", progress: { ...originalItems[0]!.progress, succeeded: 2 } };
    stream.emit("update", { task: latest });

    expect(state.items.value).toBe(originalItems);
    expect(commits).toBe(0);
    await vi.advanceTimersByTimeAsync(99);
    expect(commits).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    await flushPromises();

    expect(commits).toBe(1);
    expect(state.items.value).not.toBe(originalItems);
    expect(state.items.value[0]?.stage).toBe("storage");
    expect(state.items.value[0]?.progress.succeeded).toBe(2);
    expect(state.items.value[99]?.stage).toBe("indexing");
    expect(listTasks).toHaveBeenCalledTimes(baselineLoads);

    stopWatching();
    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("invalidates a pending page patch before a filter mutation and reloads structural membership changes", async () => {
    const tasks = Array.from({ length: 100 }, (_, index) => task(index));
    const { state, wrapper, stream } = await loadHundredTasks(tasks);
    vi.useFakeTimers();
    const firstTask = state.items.value[0]!;
    stream.emit("update", { task: { ...firstTask, stage: "stale-buffer" } });
    state.setKindFilter("url_batch");
    await flushPromises();
    await vi.advanceTimersByTimeAsync(1_100);
    expect(state.items.value.every((item) => item.kind === "url_batch")).toBe(true);
    expect(state.items.value.some((item) => item.task_id === firstTask.task_id)).toBe(false);

    const selected = state.items.value[0]!;
    tasks.find((item) => item.task_id === selected.task_id)!.status = "succeeded";
    stream.emit("update", { task: { ...selected, status: "succeeded" } });
    expect(state.items.value[0]?.status).not.toBe("succeeded");
    await vi.advanceTimersByTimeAsync(1_000);
    await flushPromises();
    expect(state.items.value.some((item) => item.status === "succeeded")).toBe(false);
    expect(listTasks).toHaveBeenCalledTimes(5);

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("limits bulk requests and reports captured rerun successes after live and list state change", async () => {
    const tasks = Array.from({ length: 100 }, (_, index) => task(index, index % 2 === 0 ? "failed" : "cancelled"));
    const { state, wrapper, stream } = await loadHundredTasks(tasks);
    let active = 0;
    let maximumActive = 0;
    const submit = (taskId: string, action: "retry" | "rerun") => new Promise<unknown>((resolve, reject) => {
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      setTimeout(() => {
        active -= 1;
        const current = tasks.find((item) => item.task_id === taskId)!;
        if (taskId === "task-000") {
          reject(new Error("no retryable failed items"));
        } else if (taskId === "task-099") {
          reject(new Error("rerun rejected"));
        } else {
          current.status = "queued";
          resolve(action === "retry"
            ? { retried_items: 1, task: { task_id: taskId, item_ids: [] } }
            : { task: { task_id: taskId, item_ids: [] } });
        }
      }, 500);
    });
    retryTask.mockImplementation((taskId) => submit(taskId, "retry") as never);
    rerunTask.mockImplementation((taskId) => submit(taskId, "rerun") as never);

    vi.useFakeTimers();
    const recovery = state.recoverAll();
    const rerunTarget = state.items.value[99]!;
    tasks.find((item) => item.task_id === rerunTarget.task_id)!.status = "failed";
    stream.emit("update", { task: { ...rerunTarget, status: "failed" } });
    await vi.advanceTimersByTimeAsync(100);
    expect(state.items.value[99]?.status).toBe("failed");
    await vi.runAllTimersAsync();
    await recovery;

    expect(maximumActive).toBe(4);
    expect(retryTask).toHaveBeenCalledTimes(50);
    expect(rerunTask).toHaveBeenCalledTimes(50);
    expect(state.recoverableCount.value).toBe(2);
    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      title: "98 succeeded · 1 skipped · 1 failed",
      description: "Resumed 49 tasks in place; showing the processing list",
    }));

    state.stopLiveUpdates();
    wrapper.unmount();
  });
});
