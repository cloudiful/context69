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
const cancelTask = vi.spyOn(apiClient, "cancelTask");
const getTaskMaintenance = vi.spyOn(apiClient, "getTaskMaintenance");
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
    cancelTask.mockReset().mockResolvedValue(undefined);
    getTaskMaintenance.mockReset().mockResolvedValue(maintenanceOverview as never);
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
    expect(wrapper.text()).toContain("docling");
    expect(wrapper.text()).toContain("Waiting");
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
    const getTaskItems = vi.spyOn(apiClient, "getTaskItems").mockResolvedValue({
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
});
