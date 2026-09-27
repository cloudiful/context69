import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { useI18n } from "vue-i18n";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import type { AppLocale } from "../i18n/locale";
import { createTestI18n } from "../test-utils/i18n";
import { useProcessingQueue } from "./use-processing-queue";

const mocks = vi.hoisted(() => ({ addToast: vi.fn() }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.addToast }) }));

const confirmMocks = vi.hoisted(() => ({ require: vi.fn() }));
vi.mock("./use-app-confirm", () => ({ useAppConfirm: () => ({ require: confirmMocks.require }) }));

const listTasks = vi.spyOn(apiClient, "listTasks");
const cancelTask = vi.spyOn(apiClient, "cancelTask");

const activeTask: TaskResponse = {
  task_id: "active-task-id",
  kind: "file_batch",
  origin: "manual",
  status: "running",
  group_path: "research",
  source_key: null,
  stage: "processing",
  waiting_reason: null,
  dependency_key: null,
  progress: { total: 1, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:00:30Z",
  finished_at: null,
  updated_at: "2026-07-20T00:01:00Z",
};

function page(items: TaskResponse[] = [activeTask]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

function mountQueue(locale: AppLocale) {
  let state!: ReturnType<typeof useProcessingQueue>;
  const wrapper = mount(defineComponent({
    setup() {
      const { t } = useI18n();
      state = useProcessingQueue({ t });
      return {};
    },
    template: "<div />",
  }), { global: { plugins: [createTestI18n(locale)] } });
  return { state, wrapper };
}

// The cancel toast keys were referenced by the queue actions before the en and
// zh-CN catalogs defined them, so the success title and both failure fallbacks
// must resolve to real messages rather than the raw key path.
const locales = [
  { locale: "en", successTitle: "Task cancelled", failureMessage: "Failed to cancel the task", bulkFailureMessage: "Failed to cancel active tasks" },
  { locale: "zh-CN", successTitle: "任务已取消", failureMessage: "取消任务失败", bulkFailureMessage: "取消活动任务失败" },
] as const;

describe.each(locales)("useProcessingQueue cancel feedback ($locale)", ({ locale, successTitle, failureMessage, bulkFailureMessage }) => {
  beforeEach(() => {
    mocks.addToast.mockReset();
    confirmMocks.require.mockReset();
    listTasks.mockReset().mockResolvedValue(page() as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
  });

  it("reports the translated cancel success toast for an active task", async () => {
    const { state, wrapper } = mountQueue(locale);
    await flushPromises();

    await state.cancelTask(activeTask);

    expect(cancelTask).toHaveBeenCalledWith("active-task-id");
    expect(mocks.addToast).toHaveBeenCalledWith({
      color: "success",
      title: successTitle,
      description: "active-task-id",
      duration: 2500,
    });
    wrapper.unmount();
  });

  it("falls back to the translated cancel failure message when the error carries no detail", async () => {
    cancelTask.mockRejectedValueOnce(new Error(""));
    const { state, wrapper } = mountQueue(locale);
    await flushPromises();

    await state.cancelTask(activeTask);

    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      color: "error",
      description: failureMessage,
    }));
    // The failed mutation must not refresh or release the task selection early.
    expect(listTasks).toHaveBeenCalledTimes(1);
    expect(state.actionTaskIds.value).toEqual([]);
    wrapper.unmount();
  });

  it("falls back to the translated bulk cancel failure message when a request throws while the batch is built", async () => {
    // `cancelActive` builds the request promises inside its `try`, so a
    // synchronous throw reaches the catch before `Promise.allSettled` runs.
    cancelTask.mockImplementationOnce(() => {
      throw new Error("");
    });
    const { state, wrapper } = mountQueue(locale);
    await flushPromises();

    expect(state.activeCount.value).toBe(1);
    await state.cancelActive();

    expect(cancelTask).toHaveBeenCalledWith("active-task-id");
    expect(mocks.addToast).toHaveBeenCalledTimes(1);
    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      color: "error",
      description: bulkFailureMessage,
    }));
    // The failed batch must not refresh the queue and must release the bulk lock.
    expect(listTasks).toHaveBeenCalledTimes(1);
    expect(state.bulkAction.value).toBeNull();
    wrapper.unmount();
  });
});
