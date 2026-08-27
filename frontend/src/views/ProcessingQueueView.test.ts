import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import { apiClient, type TaskMaintenanceOverview, type TaskResponse } from "../services/api";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const recoverDoclingTask = vi.spyOn(apiClient, "recoverDoclingTask");
const cancelTask = vi.spyOn(apiClient, "cancelTask");
const getTaskMaintenance = vi.spyOn(apiClient, "getTaskMaintenance");
const getTaskItems = vi.spyOn(apiClient, "getTaskItems");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
const addToast = vi.fn();
const useToast = vi.spyOn(nuxtUiComposables, "useToast");

const row: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  origin: "manual",
  status: "waiting",
  group_path: "research",
  source_key: null,
  stage: "docling",
  waiting_reason: "dependency",
  dependency_key: "docling",
  progress: { total: 1, queued: 0, running: 0, waiting: 1, succeeded: 0, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:01:00Z",
  finished_at: null,
  updated_at: "2026-07-20T00:02:00Z",
};

const failedRow: TaskResponse = {
  ...row,
  status: "failed",
  waiting_reason: null,
  dependency_key: null,
  failure_stage: "indexing",
  error_summary: "Qdrant unavailable",
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 0 },
  finished_at: "2026-07-20T00:02:00Z",
};

const waitingQdrantRow: TaskResponse = {
  ...row,
  task_id: "waiting-qdrant-task-id",
  dependency_key: "qdrant",
};

const waitingEmbeddingRow: TaskResponse = {
  ...row,
  task_id: "waiting-embedding-task-id",
  stage: "embedding",
  dependency_key: "embedding",
};

const waitingLegacyEmbeddingRow: TaskResponse = {
  ...row,
  task_id: "waiting-legacy-embedding-task-id",
  stage: "embedding",
  dependency_key: "embedding_vector",
};

const waitingUnknownDependencyRow: TaskResponse = {
  ...row,
  task_id: "waiting-unknown-dependency-task-id",
  stage: "storage",
  dependency_key: "custom_storage",
};

function response(items: TaskResponse[]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

const maintenanceOverview: TaskMaintenanceOverview = {
  settings: { cleanup_enabled: true, retention_days: 30, updated_at: "2026-07-20T00:00:00Z" },
  stats: { total: 40, queued: 2, running: 1, waiting: 3, succeeded: 25, failed: 5, cancelled: 4, active: 6, expired_terminal: 12 },
};

async function mountQueue() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
  });
  await router.push("/processing-queue");
  await router.isReady();
  return mount(ProcessingQueueView, {
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en"), router] },
  });
}

describe("ProcessingQueueView", () => {
  beforeEach(() => {
    setGuest();
    listTasks.mockReset().mockResolvedValue(response([row]) as never);
    retryTask.mockReset().mockResolvedValue({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    recoverDoclingTask.mockReset().mockResolvedValue({ recovered: { task_id: "task-id" } } as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
    getTaskMaintenance.mockReset().mockResolvedValue(maintenanceOverview as never);
    getTaskItems.mockReset();
    useOverlay.mockReset().mockReturnValue({
      create: () => ({ open: async () => true }),
    } as never);
    addToast.mockReset();
    useToast.mockReset().mockReturnValue({ add: addToast } as never);
  });

  it("shows waiting stage and dependency reason", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Docling");
    expect(wrapper.text()).toContain("Docling");
    expect(wrapper.text()).toContain("Waiting");
    wrapper.unmount();
  });

  it("renders the localized Qdrant label for the waiting dependency column", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingQdrantRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Dependency: Qdrant");
    expect(wrapper.text()).toContain("waiting-qdrant-task-id");
    wrapper.unmount();
  });

  it("renders the localized Embedding label for the waiting dependency column", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingEmbeddingRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Dependency: Embedding");
    expect(wrapper.text()).not.toContain("dependency_key: embedding");
    wrapper.unmount();
  });

  it("renders the legacy embedding_vector dependency as the Embedding label", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingLegacyEmbeddingRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Dependency: Embedding");
    expect(wrapper.text()).not.toContain("embedding_vector");
    wrapper.unmount();
  });

  it("keeps unknown dependency keys visible as their raw value", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingUnknownDependencyRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Dependency: custom_storage");
    wrapper.unmount();
  });

  it("renders a library dependency filter select alongside the existing filters", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    const labels = ["Task status", "Task type", "Task stage", "Waiting reason", "Library dependency"];
    for (const label of labels) {
      expect(wrapper.find(`[aria-label="${label}"]`).exists()).toBe(true);
    }
    wrapper.unmount();
  });

  it("forwards dependency_key through listTasks when the dependency filter changes", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    const queue = (wrapper.vm as unknown as { queue: { setDependencyKeyFilter(value: string | null): void } }).queue;
    queue.setDependencyKeyFilter("qdrant");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ dependencyKey: "qdrant" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    queue.setDependencyKeyFilter(null);
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ dependencyKey: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("lets the table own horizontal scrolling without a page-level wrapper", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.find('[data-testid="processing-queue-table-scroll"]').exists()).toBe(false);
    const table = wrapper.find('[data-testid="processing-queue-table"] table');
    expect(table.exists()).toBe(true);
    expect(table.classes()).toContain("min-w-[88rem]");
    const root = wrapper.find("section");
    expect(root.classes()).toContain("overflow-x-hidden");
    expect(root.classes()).not.toContain("overflow-x-auto");
    wrapper.unmount();
  });

  it("shows a top-aligned empty state instead of a table when no tasks are visible", async () => {
    listTasks.mockResolvedValue(response([]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("No tasks");
    expect(wrapper.find("table").exists()).toBe(false);
    wrapper.unmount();
  });

  it("expands a task row and loads its items", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "item-id",
          status: "failed",
          stage: "indexing",
          attempt_count: 3,
          error_message: "Qdrant unavailable",
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    expect(expandButton).toBeDefined();
    await expandButton!.trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledWith("task-id", expect.objectContaining({ limit: 100 }));
    expect(wrapper.text()).toContain("item-id");
    expect(wrapper.text()).toContain("Qdrant unavailable");
    expect(wrapper.text()).toContain("3 attempts");
    wrapper.unmount();
  });

  it("retries a failed task through the unified endpoint", async () => {
    listTasks
      .mockResolvedValueOnce(response([failedRow]) as never)
      .mockResolvedValueOnce(response([row]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry"));
    expect(retryButton).toBeDefined();
    await retryButton!.trigger("click");
    await flushPromises();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("shows retry for a cancelled task with failed items", async () => {
    const cancelledFailedRow: TaskResponse = {
      ...failedRow,
      task_id: "cancelled-task-id",
      status: "cancelled",
      progress: { total: 2, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 1 },
    };
    listTasks.mockResolvedValue(response([cancelledFailedRow]) as never);

    const wrapper = await mountQueue();
    await flushPromises();

    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry"));
    expect(retryButton).toBeDefined();
    wrapper.unmount();
  });

  it("cancels a waiting task through the unified endpoint", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    const cancelButton = wrapper.findAll("button").find((button) => button.attributes("aria-label")?.includes("Cancel"));
    expect(cancelButton).toBeDefined();
    await cancelButton!.trigger("click");
    await flushPromises();

    expect(cancelTask).toHaveBeenCalledWith("task-id");
    wrapper.unmount();
  });

  it("hides maintenance controls from non-admin users", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);
    expect(getTaskMaintenance).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows a compact admin toolbar and opens cleanup settings in a modal", async () => {
    setAuthenticatedUser({ is_admin: true });
    const wrapper = await mountQueue();
    await flushPromises();

    expect(getTaskMaintenance).toHaveBeenCalledOnce();
    const toolbar = wrapper.find('[data-testid="task-maintenance-toolbar"]');
    expect(toolbar.exists()).toBe(true);
    expect(toolbar.classes()).toContain("border-t");
    expect(toolbar.text()).toContain("Task history maintenance");
    expect(toolbar.text()).toContain("Total tasks: 40");
    expect(toolbar.text()).toContain("Active: 6");
    expect(toolbar.text()).toContain("Expired history: 12");

    // Inline settings form must not reserve page height; it lives in the modal.
    expect(wrapper.find('[data-testid="maintenance-cleanup-toggle"]').exists()).toBe(false);

    const settingsButton = toolbar.find('[data-testid="maintenance-settings-button"]');
    expect(settingsButton.attributes("aria-label")).toBe("Cleanup settings");

    expect(document.body.textContent).not.toContain("Retention (days)");
    await settingsButton.trigger("click");
    await flushPromises();

    expect(document.body.textContent).toContain("Cleanup settings");
    expect(document.body.textContent).toContain("Auto-cleanup of expired tasks");
    expect(document.body.textContent).toContain("Retention (days)");
    expect(document.body.textContent).toContain("Save");
    wrapper.unmount();
  });

  it("restores persisted settings when the modal is reopened after cancel", async () => {
    setAuthenticatedUser({ is_admin: true });
    const wrapper = await mountQueue();
    await flushPromises();

    await wrapper.find('[data-testid="maintenance-settings-button"]').trigger("click");
    await flushPromises();

    const toggle = document.querySelector('[data-testid="maintenance-cleanup-toggle"]');
    expect(toggle).not.toBeNull();
    expect(toggle!.getAttribute("aria-checked")).toBe("true");

    toggle!.dispatchEvent(new Event("click", { bubbles: true }));
    await flushPromises();
    expect(toggle!.getAttribute("aria-checked")).toBe("false");

    const cancelButton = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Cancel",
    );
    expect(cancelButton).toBeDefined();
    cancelButton!.click();
    await flushPromises();
    expect(document.body.textContent).not.toContain("Retention (days)");

    // Reopening must discard the discarded draft and restart from persisted values.
    await wrapper.find('[data-testid="maintenance-settings-button"]').trigger("click");
    await flushPromises();

    const reopenedToggle = document.querySelector('[data-testid="maintenance-cleanup-toggle"]');
    expect(reopenedToggle!.getAttribute("aria-checked")).toBe("true");
    wrapper.unmount();
  });

  it("renders a retryable failed item with a Retry button that calls the task-scoped retry endpoint", async () => {
    listTasks
      .mockResolvedValueOnce(response([failedRow]) as never)
      .mockResolvedValueOnce(response([row]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "item-id",
          status: "failed",
          stage: "indexing",
          attempt_count: 3,
          error_message: "Qdrant unavailable",
          failure_stage: "indexing",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    expect(expandButton).toBeDefined();
    await expandButton!.trigger("click");
    await flushPromises();

    const itemRetry = wrapper.findAll("button").filter((button) => button.text().includes("Retry file"));
    expect(itemRetry.length).toBeGreaterThanOrEqual(2);
    await itemRetry[itemRetry.length - 1]!.trigger("click");
    await flushPromises();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(getTaskItems).toHaveBeenCalledWith("task-id", expect.objectContaining({ limit: 100 }));
    expect(recoverDoclingTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("routes Qdrant/indexing/embedding failed items through retry and never through Docling recovery", async () => {
    listTasks.mockResolvedValue(response([failedRow]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "qdrant-item",
          status: "failed",
          stage: "indexing",
          attempt_count: 1,
          error_message: "Qdrant unreachable",
          failure_stage: "indexing",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
        {
          item_id: "embedding-item",
          status: "failed",
          stage: "embedding",
          attempt_count: 1,
          error_message: "Embedding failed",
          failure_stage: "embedding",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
        {
          item_id: "qdrant-only-item",
          status: "failed",
          stage: "embedding",
          attempt_count: 1,
          error_message: "qdrant: connection refused",
          failure_stage: "qdrant",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    setAuthenticatedUser({ is_admin: true });
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    const doclingButtons = wrapper.findAll("button").filter((button) => button.text().includes("Docling"));
    expect(doclingButtons).toHaveLength(0);
    const retryButtons = wrapper.findAll("button").filter((button) => button.text().includes("Retry file"));
    expect(retryButtons.length).toBeGreaterThanOrEqual(3);

    expect(recoverDoclingTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows admin-only Docling recovery for failed Docling items and routes through recoverDoclingTask", async () => {
    listTasks.mockResolvedValueOnce(response([failedRow]) as never).mockResolvedValueOnce(response([row]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "docling-item",
          status: "failed",
          stage: "docling_poll",
          attempt_count: 1,
          error_message: "Docling submission outcome is uncertain",
          failure_stage: "docling_poll",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    setAuthenticatedUser({ is_admin: true });
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    const doclingButton = wrapper.findAll("button").find((button) => button.text().includes("Docling"));
    expect(doclingButton).toBeDefined();
    await doclingButton!.trigger("click");
    await flushPromises();

    expect(recoverDoclingTask).toHaveBeenCalledWith("task-id", expect.objectContaining({ reason: expect.stringContaining("Docling") }));
    expect(retryTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("hides Docling recovery for non-admin users and routes failed Docling items through retry", async () => {
    listTasks.mockResolvedValue(response([failedRow]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "docling-item",
          status: "failed",
          stage: "docling_poll",
          attempt_count: 1,
          error_message: "Docling submission outcome is uncertain",
          failure_stage: "docling_poll",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    // Guest user (non-admin).
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    const doclingButtons = wrapper.findAll("button").filter((button) => button.text().includes("Docling"));
    expect(doclingButtons).toHaveLength(0);
    wrapper.unmount();
  });

  it("hides the per-item action for non-actionable items", async () => {
    listTasks.mockResolvedValue(response([failedRow]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "non-retryable-item",
          status: "failed",
          stage: "indexing",
          attempt_count: 1,
          error_message: "Hard failure",
          failure_stage: "indexing",
          retryable: false,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
        {
          item_id: "succeeded-item",
          status: "succeeded",
          stage: "indexing",
          attempt_count: 1,
          error_message: null,
          failure_stage: null,
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("non-retryable-item");
    expect(wrapper.text()).toContain("succeeded-item");

    const retryButtons = wrapper.findAll("button").filter((button) => button.text().includes("Retry file"));
    expect(retryButtons).toHaveLength(1);
    expect(retryButtons[0]!.text()).toContain("Retry file");
    wrapper.unmount();
  });

  it("prevents duplicate row and cell retry requests for the same task", async () => {
    let resolveRetry: (() => void) | null = null;
    retryTask.mockReset().mockImplementationOnce(() => new Promise((resolve) => {
      resolveRetry = () => resolve({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    }));
    listTasks
      .mockResolvedValueOnce(response([failedRow]) as never)
      .mockResolvedValueOnce(response([row]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "item-id",
          status: "failed",
          stage: "indexing",
          attempt_count: 3,
          error_message: "Qdrant unavailable",
          failure_stage: "indexing",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    const retryButtons = wrapper.findAll("button").filter((button) => button.text().includes("Retry file"));
    expect(retryButtons.length).toBeGreaterThanOrEqual(2);
    await retryButtons[0]!.trigger("click");
    await flushPromises();

    expect(retryTask).toHaveBeenCalledTimes(1);

    // The cell button must reflect the in-flight row retry as disabled/loading.
    const stillPending = retryButtons[retryButtons.length - 1]!;
    const disabledAttr = stillPending.attributes("disabled");
    expect(disabledAttr !== undefined || stillPending.classes().some((cls) => cls.includes("disabled"))).toBe(true);

    resolveRetry!();
    await flushPromises();
    wrapper.unmount();
  });

  it("exposes a retry-load button when the item list fails and reuses getTaskItems", async () => {
    listTasks.mockReset().mockResolvedValue(response([failedRow]) as never);
    getTaskItems.mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce({
        items: [
          {
            item_id: "item-id",
            status: "failed",
            stage: "indexing",
            attempt_count: 1,
            error_message: "Qdrant unavailable",
            failure_stage: "indexing",
            retryable: true,
            created_at: "2026-07-20T00:01:00Z",
            updated_at: "2026-07-20T00:02:00Z",
          },
        ],
        next_cursor: undefined,
      } as never);

    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("Failed to load task items");
    expect(getTaskItems).toHaveBeenCalledTimes(1);

    const retryLoad = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Retry loading items");
    expect(retryLoad).toBeDefined();
    await retryLoad!.trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(getTaskItems).toHaveBeenLastCalledWith("task-id", expect.objectContaining({ limit: 100 }));
    expect(wrapper.text()).toContain("item-id");
    wrapper.unmount();
  });

  it("surfaces the bounded upstream message and a stable HTTP status when item loading fails with an HTTP error", async () => {
    listTasks.mockReset().mockResolvedValue(response([failedRow]) as never);
    const longMessage = `Qdrant unavailable: ${"x".repeat(400)}`;
    const { ApiError } = await import("../services/api/api-core");
    getTaskItems.mockRejectedValueOnce(
      new ApiError(longMessage, 503),
    );

    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalled();
    const errorSpan = wrapper.find('[aria-label="Failed to load task items"]');
    expect(errorSpan.exists()).toBe(true);
    expect(wrapper.text()).toContain("· 503");
    expect(errorSpan.attributes("title")).toBeDefined();
    expect((errorSpan.attributes("title") ?? "").length).toBeLessThanOrEqual(240);
    expect((errorSpan.attributes("title") ?? "")).toContain("Qdrant unavailable");
    wrapper.unmount();
  });
});
