import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { useProcessingQueue } from "./use-processing-queue";

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const rerunTask = vi.spyOn(apiClient, "rerunTask");
const recoverDoclingTask = vi.spyOn(apiClient, "recoverDoclingTask");
const queueDoclingRecovery = vi.spyOn(apiClient, "queueDoclingRecovery");
const cancelTask = vi.spyOn(apiClient, "cancelTask");

const failedTask: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  origin: "manual",
  status: "failed",
  group_path: "research",
  source_key: null,
  stage: "indexing",
  waiting_reason: null,
  dependency_key: null,
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 0 },
  failure_stage: "indexing",
  error_summary: "Embedding failed",
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:01:00Z",
  finished_at: "2026-07-20T00:02:00Z",
  updated_at: "2026-07-20T00:02:00Z",
};

const cancelledTask: TaskResponse = {
  ...failedTask,
  task_id: "cancelled-task-id",
  status: "cancelled",
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 0, cancelled: 1 },
  failure_stage: null,
  error_summary: null,
  finished_at: "2026-07-20T00:03:00Z",
};

const waitingTask: TaskResponse = {
  ...failedTask,
  task_id: "waiting-task-id",
  status: "waiting",
  stage: "docling",
  waiting_reason: "dependency",
  dependency_key: "docling",
  progress: { total: 1, queued: 0, running: 0, waiting: 1, succeeded: 0, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  finished_at: null,
};

const waitingDoclingPollTask: TaskResponse = {
  ...waitingTask,
  task_id: "waiting-docling-poll-task-id",
  stage: "docling_poll",
};

function page(items: TaskResponse[] = [failedTask]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

const failedDoclingTask: TaskResponse = {
  ...failedTask,
  task_id: "failed-docling-task-id",
  stage: "docling_poll",
  failure_stage: "docling_poll",
  error_summary: "Docling submission outcome is uncertain",
};

describe("useProcessingQueue", () => {
  beforeEach(() => {
    listTasks.mockReset().mockResolvedValue(page() as never);
    retryTask.mockReset().mockResolvedValue({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    rerunTask.mockReset().mockResolvedValue({ task: { task_id: "new-task-id", item_ids: [] } } as never);
    recoverDoclingTask.mockReset().mockResolvedValue({ recovered: { task_id: "waiting-docling-poll-task-id" } } as never);
    queueDoclingRecovery.mockReset().mockResolvedValue({ queued: { task_id: "failed-docling-task-id", item_id: "item-id", stage: "docling", queued_at: "2026-07-20T00:04:00Z", already_queued: false } } as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
  });

  function mountState() {
    let state!: ReturnType<typeof useProcessingQueue>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProcessingQueue({ t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
    return { state, wrapper };
  }

  it("loads tasks and applies status, stage, search, and pagination filters", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(listTasks).toHaveBeenCalledOnce();
    expect(listTasks).toHaveBeenLastCalledWith(
      {
        page: 1,
        pageSize: 25,
        query: "",
        kind: null,
        status: null,
        stage: null,
        waitingReason: null,
        dependencyKey: null,
      },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.setStatusFilter("waiting");
    state.setStageFilter("docling");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "waiting", stage: "docling" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.searchInput.value = "embedding";
    state.submitSearch();
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "embedding" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.changePage(2);
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2, status: "waiting", stage: "docling" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("forwards the dependency filter as dependency_key on listTasks", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setDependencyKeyFilter("qdrant");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ dependencyKey: "qdrant" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.setDependencyKeyFilter(null);
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ dependencyKey: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("retries a failed task through /retry", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([waitingTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(failedTask);

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(rerunTask).not.toHaveBeenCalled();
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.items.value[0].status).toBe("waiting");
    wrapper.unmount();
  });

  it("resubmits a cancelled task through /rerun", async () => {
    listTasks
      .mockResolvedValueOnce(page([cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(cancelledTask);

    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(retryTask).not.toHaveBeenCalled();
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("does not count a waiting Docling poll task as recoverable while the remote job is still active", async () => {
    listTasks.mockResolvedValueOnce(page([waitingDoclingPollTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.isDoclingRecoveryTask(waitingDoclingPollTask)).toBe(false);
    expect(state.isRecoverableTask(waitingDoclingPollTask)).toBe(false);
    expect(state.recoverableCount.value).toBe(0);
    expect(state.doclingRecoveryCount.value).toBe(0);

    await state.recoverTask(waitingDoclingPollTask);

    expect(recoverDoclingTask).not.toHaveBeenCalled();
    expect(retryTask).not.toHaveBeenCalled();
    expect(rerunTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("uses the admin recovery endpoint for failed Docling tasks", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedDoclingTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(failedDoclingTask);

    expect(recoverDoclingTask).toHaveBeenCalledWith(
      "failed-docling-task-id",
      { reason: "manual recovery from the processing queue" },
    );
    expect(queueDoclingRecovery).not.toHaveBeenCalled();
    expect(retryTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("refreshes and surfaces the quarantine path when immediate recovery hits an uncertain 409", async () => {
    const { ApiError } = await import("../services/api/api-core");
    listTasks
      .mockResolvedValueOnce(page([failedDoclingTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    recoverDoclingTask.mockRejectedValueOnce(
      new ApiError("Docling submission outcome is uncertain; quarantine the stale submitting job before recovery", 409),
    );
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(failedDoclingTask);

    expect(recoverDoclingTask).toHaveBeenCalledTimes(1);
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("routes item-level Docling recovery through recoverDoclingFromItem even when the task is not task-level docling", async () => {
    const nonDoclingFailedTask: TaskResponse = {
      ...failedTask,
      task_id: "non-docling-task-id",
      failure_stage: "indexing",
    };
    listTasks
      .mockResolvedValueOnce(page([nonDoclingFailedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.isDoclingRecoveryTask(nonDoclingFailedTask)).toBe(false);
    await state.recoverDoclingFromItem(nonDoclingFailedTask);

    expect(recoverDoclingTask).toHaveBeenCalledWith(
      "non-docling-task-id",
      { reason: expect.stringContaining("Docling") },
    );
    expect(retryTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("rejects a duplicate item-level Docling recovery while another task-scoped action is in flight", async () => {
    let resolveRetry: (() => void) | null = null;
    retryTask.mockReset().mockImplementationOnce(() => new Promise((resolve) => {
      resolveRetry = () => resolve({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    }));
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    const inFlight = state.recoverTask(failedTask);
    await state.recoverDoclingFromItem(failedTask);

    expect(recoverDoclingTask).not.toHaveBeenCalled();
    expect(retryTask).toHaveBeenCalledTimes(1);

    resolveRetry!();
    await inFlight;
    wrapper.unmount();
  });

  it("counts failed and cancelled tasks as recoverable, ignoring succeeded and active ones", async () => {
    const succeededTask: TaskResponse = {
      ...failedTask,
      task_id: "succeeded-task-id",
      status: "succeeded",
      progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
    };
    listTasks.mockResolvedValueOnce(page([failedTask, cancelledTask, waitingTask, succeededTask, waitingDoclingPollTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.isRecoverableTask(failedTask)).toBe(true);
    expect(state.isRecoverableTask(cancelledTask)).toBe(true);
    expect(state.isRecoverableTask(waitingTask)).toBe(false);
    expect(state.isRecoverableTask(waitingDoclingPollTask)).toBe(false);
    expect(state.isRecoverableTask(succeededTask)).toBe(false);
    expect(state.isDoclingRecoveryTask(waitingDoclingPollTask)).toBe(false);
    expect(state.isDoclingRecoveryTask(failedDoclingTask)).toBe(true);
    expect(state.recoverableCount.value).toBe(2);
    expect(state.doclingRecoveryCount.value).toBe(0);
    expect(state.failedCount.value).toBe(1);
    expect(state.cancelledCount.value).toBe(1);
    expect(state.activeCount.value).toBe(2);
    wrapper.unmount();
  });

  it("cancels waiting tasks through the unified task endpoint", async () => {
    listTasks
      .mockResolvedValueOnce(page([waitingTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.cancelTask(waitingTask);

    expect(cancelTask).toHaveBeenCalledWith("waiting-task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("recovers every visible failed and cancelled task and refreshes once", async () => {
    const secondFailedTask = { ...failedTask, task_id: "task-id-2" };
    listTasks
      .mockResolvedValueOnce(page([failedTask, secondFailedTask, cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.recoverableCount.value).toBe(3);
    await state.recoverAll();

    expect(retryTask).toHaveBeenCalledTimes(2);
    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(retryTask).toHaveBeenCalledWith("task-id-2");
    expect(rerunTask).toHaveBeenCalledTimes(1);
    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("excludes waiting Docling polls from bulk recovery while keeping failed Docling tasks recoverable", async () => {
    listTasks.mockResolvedValueOnce(page([waitingDoclingPollTask, failedDoclingTask]) as never);
    const { state, wrapper } = mountState();

    await flushPromises();

    expect(state.isRecoverableTask(waitingDoclingPollTask)).toBe(false);
    expect(state.isRecoverableTask(failedDoclingTask)).toBe(true);
    expect(state.recoverableCount.value).toBe(1);
    expect(state.doclingRecoveryCount.value).toBe(1);

    await state.recoverAll();

    expect(queueDoclingRecovery).toHaveBeenCalledTimes(1);
    expect(queueDoclingRecovery).toHaveBeenCalledWith(
      "failed-docling-task-id",
      { reason: "bulk queue-only recovery from the processing queue" },
    );
    expect(queueDoclingRecovery).not.toHaveBeenCalledWith(
      "waiting-docling-poll-task-id",
      expect.anything(),
    );
    expect(recoverDoclingTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("treats an already-queued bulk response as idempotent success", async () => {
    queueDoclingRecovery.mockResolvedValueOnce({
      queued: { task_id: "failed-docling-task-id", item_id: "item-id", stage: "docling", queued_at: "2026-07-20T00:04:00Z", already_queued: true },
    } as never);
    listTasks
      .mockResolvedValueOnce(page([failedDoclingTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverAll();

    expect(queueDoclingRecovery).toHaveBeenCalledTimes(1);
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("does not trigger bulk recovery when only waiting Docling polls are visible", async () => {
    listTasks.mockResolvedValueOnce(page([waitingDoclingPollTask]) as never);
    const { state, wrapper } = mountState();

    await flushPromises();

    expect(state.recoverableCount.value).toBe(0);

    await state.recoverAll();

    expect(queueDoclingRecovery).not.toHaveBeenCalled();
    expect(recoverDoclingTask).not.toHaveBeenCalled();
    expect(retryTask).not.toHaveBeenCalled();
    expect(rerunTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("keeps refreshing and reporting when one recovery fails", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask, cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    retryTask.mockRejectedValueOnce(new Error("no retryable failed items"));
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverAll();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
});
