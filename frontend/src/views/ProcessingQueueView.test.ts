import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import { apiClient, type TaskResponse } from "../services/api";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const cancelTask = vi.spyOn(apiClient, "cancelTask");
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
  stage: "processing",
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
  failure_stage: "processing",
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
  stage: "processing",
  dependency_key: "embedding",
};

const waitingLegacyEmbeddingRow: TaskResponse = {
  ...row,
  task_id: "waiting-legacy-embedding-task-id",
  stage: "processing",
  dependency_key: "embedding_vector",
};

const waitingUnknownDependencyRow: TaskResponse = {
  ...row,
  task_id: "waiting-unknown-dependency-task-id",
  stage: "processing",
  dependency_key: "custom_storage",
};

const waitingDoclingRow: TaskResponse = {
  ...row,
  task_id: "waiting-docling-task-id",
  stage: "processing",
  waiting_reason: "docling",
  dependency_key: "docling",
};

function response(items: TaskResponse[]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

// Header filters are Nuxt UI menus portalled to the document body, so a test
// reads the open menu from there instead of from the wrapper.
function menuItems() {
  const menu = document.body.querySelector('[role="menu"]');
  expect(menu).not.toBeNull();
  return [...(menu as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]')];
}

// A nested menu entry opens its submenu with the documented keyboard gesture,
// which is also how a keyboard user reaches it.
async function openSubmenu(item: HTMLElement) {
  item.focus();
  item.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  await flushPromises();
}

async function mountQueue(locale: "en" | "zh-CN" = "en") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
  });
  await router.push("/processing-queue");
  await router.isReady();
  return mount(ProcessingQueueView, {
    global: { plugins: [testNuxtUiPlugin, createTestI18n(locale), router] },
  });
}

describe("ProcessingQueueView", () => {
  beforeEach(() => {
    setGuest();
    listTasks.mockReset().mockResolvedValue(response([row]) as never);
    retryTask.mockReset().mockResolvedValue({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
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

    // Waiting renders 1:1 (the badge fix removed the queued/waiting collapse)
    // and the collapsed stage renders its own label.
    expect(wrapper.text()).toContain("Processing");
    expect(wrapper.text()).toContain("Dependency: Docling");
    expect(wrapper.text()).toContain("Waiting");
    wrapper.unmount();
  });

  it("shows per-status parent progress counts and the failure stage beside the error", async () => {
    listTasks.mockReset().mockResolvedValue(response([failedRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const progress = wrapper.find('[data-testid="queue-progress"]');
    expect(progress.exists()).toBe(true);
    expect(progress.text()).toContain("0/1");
    expect(progress.text()).toContain("Failed 1");

    const error = wrapper.find('[data-testid="queue-error"]');
    expect(error.exists()).toBe(true);
    expect(error.text()).toContain("Processing");
    expect(error.text()).toContain("Qdrant unavailable");
    wrapper.unmount();
  });

  it("shows running, waiting, and queued parent progress counts", async () => {
    const mixed: TaskResponse = {
      ...row,
      progress: { total: 10, queued: 4, running: 2, waiting: 1, succeeded: 1, failed: 2, cancelled: 0 },
    };
    listTasks.mockReset().mockResolvedValue(response([mixed]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const progress = wrapper.find('[data-testid="queue-progress"]');
    expect(progress.text()).toContain("1/10");
    expect(progress.text()).toContain("Running 2");
    expect(progress.text()).toContain("Waiting 1");
    expect(progress.text()).toContain("Queued 4");
    expect(progress.text()).toContain("Failed 2");
    wrapper.unmount();
  });

  it("renders the collapsed row by file name and document title instead of the task UUID", async () => {
    const named: TaskResponse = {
      ...row,
      task_id: "named-task-id",
      file_name: "2026-09-30-report.pdf",
      document_title: "Q3 disclosure report",
    };
    listTasks.mockReset().mockResolvedValue(response([named]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const cell = wrapper.find('[data-testid="queue-task"]');
    expect(cell.text()).toContain("2026-09-30-report.pdf");
    expect(wrapper.find('[data-testid="queue-task-title"]').text()).toBe("Q3 disclosure report");
    expect(wrapper.text()).not.toContain("named-task-id");
    wrapper.unmount();
  });

  it("falls back to the group when a task has no file name or title", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    const cell = wrapper.find('[data-testid="queue-task"]');
    expect(cell.text()).toBe("research");
    expect(wrapper.find('[data-testid="queue-task-title"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("renders the Docling waiting reason with the dependency suffix instead of a raw key", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingDoclingRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Remote conversion: Docling");
    expect(wrapper.find('[data-testid="queue-task"]').text()).toBe("research");
    expect(wrapper.text()).not.toContain("processingQueue.waitingReasons.");
    wrapper.unmount();
  });

  it("renders the Docling waiting reason in Chinese with the dependency suffix", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingDoclingRow]) as never);
    const wrapper = await mountQueue("zh-CN");
    await flushPromises();

    expect(wrapper.text()).toContain("远端转换: Docling");
    expect(wrapper.find('[data-testid="queue-task"]').text()).toBe("research");
    expect(wrapper.text()).not.toContain("processingQueue.waitingReasons.");
    wrapper.unmount();
  });

  it("offers every waiting reason and nests the dependency submenu behind the reason", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await wrapper.find('[aria-label="Waiting reason"]').trigger("click");
    await flushPromises();

    const labels = menuItems().map((el) => el.textContent?.trim() ?? "");
    for (const label of ["All waiting reasons", "Dependency", "Backoff", "Remote conversion"]) {
      expect(labels.some((text) => text.endsWith(label))).toBe(true);
    }

    // "Dependency unavailable" is a nested entry: the submenu carries the
    // library dependency that caused the wait.
    const dependency = menuItems().find((el) => el.textContent?.trim().endsWith("Dependency"));
    expect(dependency?.getAttribute("aria-haspopup")).toBe("menu");
    await openSubmenu(dependency!);

    const submenu = [...document.body.querySelectorAll('[role="menu"]')].pop() as HTMLElement;
    const submenuLabels = [...submenu.querySelectorAll('[role="menuitem"]')].map((el) => el.textContent?.trim());
    expect(submenuLabels).toEqual(["All dependencies", "S3", "Docling", "Embedding", "Qdrant"]);

    const qdrant = [...submenu.querySelectorAll<HTMLElement>('[role="menuitem"]')]
      .find((el) => el.textContent?.trim() === "Qdrant");
    qdrant!.click();
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ waitingReason: "dependency", dependencyKey: "qdrant" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("clears the dependency narrowing when a plain waiting reason is chosen", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await wrapper.find('[aria-label="Waiting reason"]').trigger("click");
    await flushPromises();
    const docling = menuItems().find((el) => el.textContent?.trim().endsWith("Remote conversion"));
    docling!.click();
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ waitingReason: "docling", dependencyKey: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("renders entry task stages with localized labels instead of raw keys", async () => {
    const stages = ["download", "storage", "sync", "delete", "translation", "indexing"];
    listTasks.mockReset().mockResolvedValue(
      response(stages.map((stage, index) => ({ ...row, task_id: `stage-task-${index}`, stage }))) as never,
    );
    const wrapper = await mountQueue();
    await flushPromises();

    for (const label of ["Download", "Storage", "Sync", "Delete", "Translation", "Indexing"]) {
      expect(wrapper.text()).toContain(label);
    }
    expect(wrapper.text()).not.toContain("processingQueue.stages.");
    wrapper.unmount();
  });

  it("falls back to Unknown for unknown task stages", async () => {
    listTasks.mockReset().mockResolvedValue(
      response([{ ...row, task_id: "unknown-stage-task", stage: "future_stage" }]) as never,
    );
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Unknown");
    expect(wrapper.text()).not.toContain("future_stage");
    expect(wrapper.text()).not.toContain("processingQueue.stages.");
    wrapper.unmount();
  });

  it("offers every entry stage in the stage filter menu", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await wrapper.find('[aria-label="Task stage"]').trigger("click");
    await flushPromises();

    const labels = menuItems().map((el) => el.textContent?.trim());
    for (const label of ["All stages", "Download", "Storage", "Sync", "Delete", "Translation", "Indexing", "Processing", "Finalize"]) {
      expect(labels).toContain(label);
    }

    const download = menuItems().find((el) => el.textContent?.trim() === "Download");
    expect(download).toBeDefined();
    download!.click();
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ stage: "download" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("renders the localized Qdrant label for the waiting dependency column", async () => {
    listTasks.mockReset().mockResolvedValue(response([waitingQdrantRow]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Dependency: Qdrant");
    expect(wrapper.find('[data-testid="queue-task"]').text()).toBe("research");
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

  it("renders header filter menus for the column-specific filters", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    // Column-specific filters moved from the toolbar into header menus; global
    // search stays in the toolbar. The waiting column owns one control whose
    // dependency choice is nested, so there is no second dependency button.
    const labels = ["Task status", "Task type", "Task stage", "Waiting reason"];
    for (const label of labels) {
      expect(wrapper.find(`[aria-label="${label}"]`).exists()).toBe(true);
    }
    expect(wrapper.findAll('[aria-label="Waiting reason"]')).toHaveLength(1);
    expect(wrapper.find('[aria-label="Library dependency"]').exists()).toBe(false);
    expect(wrapper.find("input[placeholder]").exists()).toBe(true);
    wrapper.unmount();
  });

  it("puts the status tabs, recovery actions, and refresh in one header row without the page heading", async () => {
    listTasks.mockReset().mockResolvedValue(response([failedRow, row]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const header = wrapper.find('[data-testid="processing-queue-header"]');
    expect(header.exists()).toBe(true);
    expect(header.find('[data-testid="processing-queue-tabs"]').exists()).toBe(true);
    expect(header.find('[aria-label="Refresh queue"]').exists()).toBe(true);
    expect(header.text()).toContain("Retry recoverable tasks");
    expect(header.text()).toContain("Cancel active");
    expect(header.find("h1").exists()).toBe(false);
    wrapper.unmount();
  });

  it("shows creation and update time columns on the processing tab", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Created");
    expect(wrapper.text()).toContain("Updated");
    expect(wrapper.find('[data-testid="queue-sort-created_at"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="queue-sort-updated_at"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("sorts by creation time through the header and sends the backend sort field", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    // Processing defaults to creation-time descending; one header click flips
    // the same field to ascending through the server-side sort contract.
    await wrapper.find('[data-testid="queue-sort-created_at"]').trigger("click");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ sortBy: "created_at", sortDirection: "asc" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    expect(wrapper.find('[data-testid="queue-sort-created_at"]').attributes("data-sort")).toBe("asc");
    wrapper.unmount();
  });

  it("offers processing statuses without the completed value in the status menu", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await wrapper.find('[aria-label="Task status"]').trigger("click");
    await flushPromises();

    const labels = menuItems().map((el) => el.textContent?.trim());
    expect(labels).toContain("All statuses");
    expect(labels).toContain("Failed");
    expect(labels).not.toContain("Succeeded");

    const failed = menuItems().find((el) => el.textContent?.trim() === "Failed");
    expect(failed).toBeDefined();
    failed!.click();
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "failed" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    // Re-selecting the active value closes the menu without a duplicate request.
    const callsAfterSelect = listTasks.mock.calls.length;
    await wrapper.find('[aria-label="Task status"]').trigger("click");
    await flushPromises();
    const failedAgain = menuItems().find((el) => el.textContent?.trim() === "Failed");
    failedAgain!.click();
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(callsAfterSelect);
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

    const scroll = wrapper.find('[data-testid="processing-queue-table-scroll"]');
    expect(scroll.exists()).toBe(true);
    expect(scroll.classes()).toContain("overflow-auto");
    expect(scroll.classes()).toContain("overscroll-contain");
    const tableRoot = wrapper.find('[data-testid="processing-queue-table"]');
    expect(tableRoot.exists()).toBe(true);
    expect(tableRoot.classes()).toContain("overflow-visible");
    const table = wrapper.find('[data-testid="processing-queue-table"] table');
    expect(table.exists()).toBe(true);
    expect(table.classes()).toContain("min-w-[88rem]");
    const root = wrapper.find("section");
    expect(root.classes()).toContain("overflow-hidden");
    expect(root.classes()).not.toContain("overflow-y-auto");
    expect(root.classes()).not.toContain("overflow-x-auto");
    wrapper.unmount();
  });

  it("bounds the table in a fill-height scroll region with pagination outside", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    const root = wrapper.find("section");
    expect(root.classes()).toContain("h-full");
    expect(root.classes()).toContain("min-h-0");
    expect(root.classes()).toContain("flex-col");
    expect(root.classes()).toContain("overflow-hidden");

    const list = wrapper.find('[data-testid="processing-queue-list"]');
    expect(list.exists()).toBe(true);
    expect(list.classes()).toContain("flex-1");
    expect(list.classes()).toContain("min-h-0");

    const scroll = wrapper.find('[data-testid="processing-queue-table-scroll"]');
    expect(scroll.exists()).toBe(true);
    expect(scroll.classes()).toContain("h-full");
    expect(scroll.classes()).toContain("overflow-auto");
    expect(scroll.classes()).toContain("overscroll-contain");
    expect(scroll.attributes("class") ?? "").toContain("min-h-[220px]");
    expect(scroll.find('[data-testid="processing-queue-table"]').exists()).toBe(true);
    expect(scroll.find('[data-testid="processing-queue-table"]').classes()).toContain("overflow-visible");

    // Pagination stays outside the vertical scroll region so it remains reachable.
    expect(wrapper.find('[aria-label="Items per page"]').exists()).toBe(true);
    expect(scroll.find('[aria-label="Items per page"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("keeps the queue toolbar outside the table scroll region", async () => {
    setAuthenticatedUser({ is_admin: true });
    const wrapper = await mountQueue();
    await flushPromises();

    const scroll = wrapper.find('[data-testid="processing-queue-table-scroll"]');
    expect(scroll.exists()).toBe(true);
    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);
    expect(scroll.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("keeps the table and filters available when no tasks are visible", async () => {
    listTasks.mockResolvedValue(response([]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("No tasks");
    expect(wrapper.find("table").exists()).toBe(true);

    await wrapper.find('[aria-label="Waiting reason"]').trigger("click");
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')).not.toBeNull();

    wrapper.unmount();
  });

  it("expands a task row and loads its items", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "item-id",
          status: "failed",
          stage: "processing",
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

  it("never renders task history maintenance controls in the normal queue", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Task history maintenance");
    expect(wrapper.text()).not.toContain("Auto-cleanup of expired tasks");
    expect(wrapper.text()).not.toContain("Purge expired");
    expect(wrapper.text()).not.toContain("Purge all");
    wrapper.unmount();
  });

  it("never renders maintenance controls for admins in the normal queue", async () => {
    setAuthenticatedUser({ is_admin: true });
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Task history maintenance");
    expect(wrapper.text()).not.toContain("Expired history");
    expect(wrapper.text()).not.toContain("Uncertain");
    expect(wrapper.text()).not.toContain("Quarantinable");
    wrapper.unmount();
  });

  it("hides history clear actions on the processing tab", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.find('[data-testid="clear-completed-button"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="clear-trash-button"]').exists()).toBe(false);
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
          stage: "processing",
          attempt_count: 3,
          error_message: "Qdrant unavailable",
          failure_stage: "processing",
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
    wrapper.unmount();
  });

  it("routes every failed item through retry and never renders a Docling recovery action", async () => {
    listTasks.mockResolvedValue(response([failedRow]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "qdrant-item",
          status: "failed",
          stage: "processing",
          attempt_count: 1,
          error_message: "Qdrant unreachable",
          failure_stage: "processing",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
        {
          item_id: "embedding-item",
          status: "failed",
          stage: "processing",
          attempt_count: 1,
          error_message: "Embedding failed",
          failure_stage: "processing",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
        {
          item_id: "qdrant-only-item",
          status: "failed",
          stage: "processing",
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
    const recoveryButtons = wrapper.findAll("button").filter((button) => button.text().includes("Recover Docling"));
    expect(recoveryButtons).toHaveLength(0);
    const retryButtons = wrapper.findAll("button").filter((button) => button.text().includes("Retry file"));
    expect(retryButtons.length).toBeGreaterThanOrEqual(3);

    wrapper.unmount();
  });

  it("retries a failed Docling-stage item through the task retry endpoint for any user", async () => {
    listTasks.mockResolvedValueOnce(response([failedRow]) as never).mockResolvedValueOnce(response([row]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "docling-item",
          status: "failed",
          stage: "processing",
          attempt_count: 1,
          error_message: "Docling conversion failed",
          failure_stage: "docling",
          retryable: true,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
      ],
      next_cursor: undefined,
    } as never);
    // Guest user (non-admin): the removed admin-only Docling recovery must not
    // exist, and the item still retries through the normal task endpoint.
    const wrapper = await mountQueue();
    await flushPromises();

    const expandButton = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Expand task items");
    await expandButton!.trigger("click");
    await flushPromises();

    const itemRetry = wrapper.findAll("button").filter((button) => button.text().includes("Retry file"));
    expect(itemRetry.length).toBeGreaterThanOrEqual(1);
    await itemRetry[itemRetry.length - 1]!.trigger("click");
    await flushPromises();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    wrapper.unmount();
  });

  it("hides the per-item action for non-actionable items", async () => {
    listTasks.mockResolvedValue(response([failedRow]) as never);
    getTaskItems.mockResolvedValue({
      items: [
        {
          item_id: "non-retryable-item",
          status: "failed",
          stage: "processing",
          attempt_count: 1,
          error_message: "Hard failure",
          failure_stage: "processing",
          retryable: false,
          created_at: "2026-07-20T00:01:00Z",
          updated_at: "2026-07-20T00:02:00Z",
        },
        {
          item_id: "succeeded-item",
          status: "succeeded",
          stage: "processing",
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
          stage: "processing",
          attempt_count: 3,
          error_message: "Qdrant unavailable",
          failure_stage: "processing",
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
            stage: "processing",
            attempt_count: 1,
            error_message: "Qdrant unavailable",
            failure_stage: "processing",
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
    // Issue 446 P2: the full upstream message stays visible (no 240 truncation).
    expect(errorSpan.attributes("title")).toBe(longMessage);
    expect((errorSpan.attributes("title") ?? "")).toContain("Qdrant unavailable");
    wrapper.unmount();
  });
});
