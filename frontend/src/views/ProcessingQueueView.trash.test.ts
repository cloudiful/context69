import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import { apiClient, type TaskResponse } from "../services/api";
import { setGuest } from "../test-utils/auth";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const listTasks = vi.spyOn(apiClient, "listTasks");
const getTaskMaintenance = vi.spyOn(apiClient, "getTaskMaintenance");
const trashTask = vi.spyOn(apiClient, "trashTask");
const restoreTask = vi.spyOn(apiClient, "restoreTask");
const deleteTask = vi.spyOn(apiClient, "deleteTask");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
const useToast = vi.spyOn(nuxtUiComposables, "useToast");

function task(overrides: Partial<TaskResponse>): TaskResponse {
  return {
    task_id: "task-id",
    kind: "file_batch",
    origin: "manual",
    status: "succeeded",
    group_path: "research",
    source_key: null,
    stage: "finalize",
    waiting_reason: null,
    dependency_key: null,
    progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-07-20T00:00:00Z",
    started_at: "2026-07-20T00:01:00Z",
    finished_at: "2026-07-20T00:02:00Z",
    updated_at: "2026-07-20T00:02:00Z",
    ...overrides,
  };
}

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

async function openTrash(wrapper: ReturnType<typeof mount>) {
  await tabButton(wrapper, "Trash")!.trigger("mousedown");
  await flushPromises();
}

describe("ProcessingQueueView trash actions", () => {
  beforeEach(() => {
    setGuest();
    listTasks.mockReset();
    getTaskMaintenance.mockReset().mockResolvedValue({} as never);
    trashTask.mockReset().mockResolvedValue(task({}) as never);
    restoreTask.mockReset().mockResolvedValue(task({ deleted_at: null }) as never);
    deleteTask.mockReset().mockResolvedValue(undefined as never);
    useOverlay.mockReset().mockReturnValue({ create: () => ({ open: async () => true }) } as never);
    useToast.mockReset().mockReturnValue({ add: vi.fn() } as never);
  });

  it("lists only trashed rows in Trash and restores one", async () => {
    const trashed = task({ task_id: "trashed-task", deleted_at: "2026-07-21T00:00:00Z" });
    listTasks.mockResolvedValue(response([trashed]) as never);

    const wrapper = await mountQueue();
    await flushPromises();
    await openTrash(wrapper);

    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "trash" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    await wrapper.find('[aria-label="Restore task"]').trigger("click");
    await flushPromises();
    expect(restoreTask).toHaveBeenCalledWith("trashed-task");
    wrapper.unmount();
  });

  it("permanently deletes a trashed task after confirmation", async () => {
    const trashed = task({ task_id: "trashed-task", deleted_at: "2026-07-21T00:00:00Z" });
    listTasks.mockResolvedValue(response([trashed]) as never);

    const wrapper = await mountQueue();
    await flushPromises();
    await openTrash(wrapper);

    await wrapper.find('[aria-label="Delete permanently"]').trigger("click");
    await flushPromises();
    expect(deleteTask).toHaveBeenCalledWith("trashed-task");
    wrapper.unmount();
  });

  it("offers move-to-trash only for terminal tasks and never for active rows", async () => {
    const active = task({ task_id: "active-task", status: "running", finished_at: null });
    const terminal = task({ task_id: "terminal-task" });
    listTasks.mockResolvedValue(response([active, terminal]) as never);

    const wrapper = await mountQueue();
    await flushPromises();

    const trashButtons = wrapper.findAll('[aria-label="Move to trash"]');
    expect(trashButtons).toHaveLength(1);

    await trashButtons[0].trigger("click");
    await flushPromises();
    expect(trashTask).toHaveBeenCalledWith("terminal-task");
    wrapper.unmount();
  });

  it("stays on the active list after trashing a task", async () => {
    const terminal = task({ task_id: "terminal-task" });
    listTasks.mockResolvedValue(response([terminal]) as never);

    const wrapper = await mountQueue();
    await flushPromises();

    await wrapper.find('[aria-label="Move to trash"]').trigger("click");
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });
});
