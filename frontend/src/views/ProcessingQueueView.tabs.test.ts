import { flushPromises, mount } from "@vue/test-utils";
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
const getTaskMaintenance = vi.spyOn(apiClient, "getTaskMaintenance");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
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

function response(items: TaskResponse[]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

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

function tabButton(wrapper: ReturnType<typeof mount>, label: string) {
  return wrapper.findAll("button").find((button) => button.text().includes(label));
}

describe("ProcessingQueueView tabs", () => {
  beforeEach(() => {
    setGuest();
    listTasks.mockReset().mockResolvedValue(response([row]) as never);
    getTaskMaintenance.mockReset().mockResolvedValue({} as never);
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
      expect.objectContaining({ view: "processing", status: null, trashed: false }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("narrows the same list to the completed view when Completed is selected", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    await tabButton(wrapper, "Completed")!.trigger("mousedown");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "completed", status: null, trashed: false }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    // The status select is fixed by the tab and no longer rendered.
    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(false);

    await tabButton(wrapper, "Processing")!.trigger("mousedown");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "processing", status: null, trashed: false }),
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
      expect.objectContaining({ view: "trash", trashed: true, status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    // Trash never exposes the live status filter.
    expect(wrapper.find('[aria-label="Task status"]').exists()).toBe(false);

    await tabButton(wrapper, "Processing")!.trigger("mousedown");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ view: "processing", trashed: false }),
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
