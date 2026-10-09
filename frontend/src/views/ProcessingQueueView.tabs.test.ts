import { DOMWrapper, flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import ProcessingQueueTabs from "../components/processing-queue/ProcessingQueueTabs.vue";
import { apiClient, type TaskResponse } from "../services/api";
import { setGuest } from "../test-utils/auth";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const listTasks = vi.spyOn(apiClient, "listTasks");
const clearTaskHistory = vi.spyOn(apiClient, "clearTaskHistory");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
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

function response(items: TaskResponse[]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

async function mountQueue() {
  document.body.innerHTML = '<div id="app-route-actions"></div>';
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
  });
  await router.push("/processing-queue");
  await router.isReady();
  return mount(ProcessingQueueView, {
    attachTo: document.body,
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en"), router] },
  });
}

function tabButton(wrapper: ReturnType<typeof mount>, label: string) {
  // Issue 730: tabs live in the teleported global header row, so resolve them
  // from the route-actions target. VTU does not traverse the teleport target,
  // so wrap the teleported button for triggering.
  const fromWrapper = wrapper.findAll("button").find((button) => button.text().includes(label));
  if (fromWrapper) return fromWrapper;
  const el = [...document.body.querySelectorAll<HTMLElement>("#app-route-actions button")].find((button) => (button.textContent ?? "").includes(label));
  if (!el) return undefined;
  return new DOMWrapper(el);
}

function routeAction(testId: string) {
  // Issue 730: header actions live in the teleported global header row.
  const el = document.body.querySelector<HTMLElement>(`#app-route-actions [data-testid="${testId}"]`);
  if (!el) return undefined;
  return new DOMWrapper(el);
}

// Header filters are Nuxt UI menus portalled to the document body, so a test
// reads the open menu from there instead of from the wrapper.
function menuItems() {
  const menu = document.body.querySelector('[role="menu"]');
  expect(menu).not.toBeNull();
  return [...(menu as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]')];
}

function headerLabels(wrapper: ReturnType<typeof mount>) {
  return wrapper.find('[data-testid="processing-queue-table"]').findAll("th").map((th) => th.text().trim());
}

describe("ProcessingQueueView tabs", () => {
  beforeEach(() => {
    setGuest();
    listTasks.mockReset().mockResolvedValue(response([row]) as never);
    clearTaskHistory.mockReset().mockResolvedValue({ deleted_count: 1 } as never);
    useOverlay.mockReset().mockReturnValue({ create: () => ({ open: async () => true }) } as never);
    useToast.mockReset().mockReturnValue({ add: vi.fn() } as never);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders enabled Processing, Completed, and Trash tabs", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(tabButton(wrapper, "Processing")).toBeDefined();
    expect(tabButton(wrapper, "Completed")).toBeDefined();
    const trash = tabButton(wrapper, "Trash");
    expect(trash).toBeDefined();
    expect(trash!.attributes("disabled")).toBeUndefined();

    // Default tab uses the typed processing view, not null status guessing.
    expect(listTasks).toHaveBeenCalledOnce();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing", status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("narrows the same list to the completed view when Completed is selected", async () => {    const wrapper = await mountQueue();
    await flushPromises();

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "completed", status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    // The status select is fixed by the tab and no longer rendered.
    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(false);

    await tabButton(wrapper, "Processing")!.trigger("mousedown");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing", status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("lists only trashed tasks when Trash is selected", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "trash", status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    // Trash never exposes the live status filter.
    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(false);

    await tabButton(wrapper, "Processing")!.trigger("mousedown");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("auto-refreshes Processing but stays idle on Completed and Trash", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });

    const wrapper = await mountQueue();
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(20_000);
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(2);

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();
    const afterSwitch = listTasks.mock.calls.length;

    vi.advanceTimersByTime(20_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(afterSwitch);

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    await flushPromises();
    const afterTrash = listTasks.mock.calls.length;

    vi.advanceTimersByTime(20_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(afterTrash);

    await tabButton(wrapper, "Processing")!.trigger("mousedown");
    await flushPromises();
    const afterReturn = listTasks.mock.calls.length;

    vi.advanceTimersByTime(20_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(afterReturn + 1);
    wrapper.unmount();
  });

  it("wires the completed tab to the user-scoped clear completed action", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(routeAction("clear-completed-button")).toBeUndefined();

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();

    const clearButton = routeAction("clear-completed-button");
    expect(clearButton).toBeDefined();
    expect(routeAction("clear-trash-button")).toBeUndefined();

    await (clearButton as DOMWrapper<HTMLElement>).trigger("click");
    await flushPromises();

    expect(clearTaskHistory).toHaveBeenCalledWith({ view: "completed" });
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("wires the trash tab to the user-scoped empty trash action", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    await flushPromises();

    const clearButton = routeAction("clear-trash-button");
    expect(clearButton).toBeDefined();
    expect(routeAction("clear-completed-button")).toBeUndefined();

    await (clearButton as DOMWrapper<HTMLElement>).trigger("click");
    await flushPromises();

    expect(clearTaskHistory).toHaveBeenCalledWith({ view: "trash" });
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "trash" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("never renders task history maintenance alongside the tabs", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();
    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    await flushPromises();
    expect(wrapper.find('[data-testid="task-maintenance-toolbar"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("labels the completed timestamp as completion time and defaults to update-time ordering", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "processing", sortBy: "created_at", sortDirection: "desc" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed", sortBy: "updated_at", sortDirection: "desc" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    expect(wrapper.text()).toContain("Completed");
    expect(wrapper.text()).toContain("Created");
    expect(wrapper.text()).not.toContain("Updated");
    expect(wrapper.find('[data-testid="queue-sort-updated_at"]').attributes("data-sort")).toBe("desc");
    wrapper.unmount();
  });

  it("hides terminal-view selectors on completed while keeping type and dependency filters", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();

    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Task stage"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Waiting reason"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Task type"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="Library dependency"]').exists()).toBe(true);

    await tabButton(wrapper, "Processing")!.trigger("mousedown");
    await flushPromises();
    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="Task stage"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("keeps column filters and creation-time sorting available in trash", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    await flushPromises();

    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Task type"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="Task stage"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="Waiting reason"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="queue-sort-created_at"]').exists()).toBe(true);
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "trash", sortBy: "created_at", sortDirection: "desc" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("renders tab-aware columns: completed hides stage/waiting/progress and keeps dependency visible", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(headerLabels(wrapper)).toEqual(expect.arrayContaining(["Stage", "Waiting", "Progress"]));
    expect(headerLabels(wrapper)).not.toContain("Library dependency");

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();

    const completedHeaders = headerLabels(wrapper);
    expect(completedHeaders).not.toContain("Stage");
    expect(completedHeaders).not.toContain("Waiting");
    expect(completedHeaders).not.toContain("Progress");
    expect(completedHeaders).toEqual(
      expect.arrayContaining(["Task", "Type", "Group", "Status", "Library dependency", "Error", "Created", "Completed", "Actions"]),
    );
    expect(wrapper.find('[data-testid="queue-sort-stage"]').exists()).toBe(false);
    // The dependency value stays visible in its own completed column.
    expect(wrapper.text()).toContain("Docling");

    // Header sorting stays server-side on the completed timestamp.
    await wrapper.find('[data-testid="queue-sort-updated_at"]').trigger("click");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed", sortBy: "updated_at", sortDirection: "asc" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    // The dependency filter forwards from the completed header menu.
    await wrapper.find('[aria-label="Library dependency"]').trigger("click");
    await flushPromises();
    const qdrant = menuItems().find((el) => el.textContent?.trim() === "Qdrant");
    expect(qdrant).toBeDefined();
    qdrant!.click();
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed", dependencyKey: "qdrant" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    await flushPromises();
    expect(headerLabels(wrapper)).toEqual(expect.arrayContaining(["Stage", "Waiting", "Progress"]));
    wrapper.unmount();
  });
});

describe("ProcessingQueueTabs", () => {
  it("emits only selectable tab values", async () => {
    const wrapper = mount(ProcessingQueueTabs, {
      props: { modelValue: "processing" as const },
      global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
    });

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    expect(wrapper.emitted("update:modelValue")?.at(-1)).toEqual(["completed"]);

    await tabButton(wrapper, "Trash")!.trigger("mousedown");
    expect(wrapper.emitted("update:modelValue")?.at(-1)).toEqual(["trash"]);
    wrapper.unmount();
  });
});
