import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import TaskItemsExpanded from "./TaskItemsExpanded.vue";
import {
  apiClient,
  type TaskDiagnoseResponse,
  type TaskItemResponse,
  type TaskItemStatus,
  type TaskResponse,
} from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const getTaskItems = vi.spyOn(apiClient, "getTaskItems");
const getTaskDiagnose = vi.spyOn(apiClient, "getTaskDiagnose");

function task(total = 2781): TaskResponse {
  return {
    task_id: "task-id",
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: "research",
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    progress: { total, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-07-20T00:01:00Z",
    started_at: "2026-07-20T00:01:00Z",
    finished_at: null,
    updated_at: "2026-07-20T00:02:00Z",
  };
}

function item(
  itemId: string,
  status: TaskItemResponse["status"] = "running",
  progress: TaskItemResponse["progress"] = null,
): TaskItemResponse {
  return {
    item_id: itemId,
    status,
    stage: "processing",
    ordinal: 0,
    attempt_count: 1,
    retryable: false,
    error_message: null,
    created_at: "2026-07-20T00:01:00Z",
    updated_at: "2026-07-20T00:02:00Z",
    progress,
  } as TaskItemResponse;
}

// A progress record as the generated schema delivers it: an observed stage,
// exact counters, and the time progress was last seen.
function indexingProgress(
  overrides: Partial<NonNullable<TaskItemResponse["progress"]>> = {},
): NonNullable<TaskItemResponse["progress"]> {
  return { stage: "indexing", attempt: 1, ...overrides };
}

function mountExpanded(entry: TaskResponse = task(), locale: "en" | "zh-CN" = "en") {
  return mount(TaskItemsExpanded, {
    props: { task: entry, isActing: false },
    global: { plugins: [testNuxtUiPlugin, createTestI18n(locale)] },
  });
}

function setItemStatusFilter(wrapper: ReturnType<typeof mountExpanded>, status: TaskItemStatus) {
  const trigger = wrapper.findComponent('[data-testid="task-items-filter"]') as unknown as {
    exists(): boolean;
    vm: { $parent: unknown };
  };
  expect(trigger.exists()).toBe(true);
  let selectVm = trigger.vm.$parent as unknown as {
    $options?: { __name?: string };
    $parent?: unknown;
    $emit?: (event: string, value: unknown) => void;
  } | null;
  while (selectVm && selectVm.$options?.__name !== "Select") {
    selectVm = selectVm.$parent as typeof selectVm;
  }
  expect(selectVm?.$emit).toBeDefined();
  selectVm!.$emit!("update:model-value", status);
}

function diagnoseFor(taskId: string): TaskDiagnoseResponse {
  return {
    task: {
      task_id: taskId,
      kind: "file_batch",
      status: "waiting",
      progress: { total: 0, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
      stage: "processing",
      waiting_reason: null,
      dependency_key: null,
      next_attempt_at: null,
      failure_stage: null,
      error_summary: null,
      lease_expires_at: null,
      created_at: "2026-07-20T00:01:00Z",
      started_at: null,
      finished_at: null,
      updated_at: "2026-07-20T00:02:00Z",
    },
    items: [],
    items_truncated: false,
    dependency_gates: [],
    consistency: {
      consistent: true,
      current_item_id: null,
      mismatches: [],
      open_attempt_count: 0,
      near_exhaustion_item_count: 0,
    },
    observed_at: "2026-07-20T00:02:00Z",
  };
}

describe("TaskItemsExpanded", () => {
  beforeEach(() => {
    getTaskItems.mockReset();
    getTaskDiagnose.mockReset();
  });

  it("reviewer round 2: unmount cancels the remaining refresh pages", async () => {
    vi.useFakeTimers();
    let resolveRefresh!: (value: Awaited<ReturnType<typeof apiClient.getTaskItems>>) => void;
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveRefresh = resolve; }))
      .mockResolvedValue({ items: [item("item-2")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    try {
      await flushPromises();
      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();
      await vi.advanceTimersByTimeAsync(3000);
      expect(getTaskItems).toHaveBeenCalledTimes(3);
      wrapper.unmount();
      resolveRefresh({ items: [item("item-1")], next_cursor: "new-cursor" } as never);
      await flushPromises();
      expect(getTaskItems).toHaveBeenCalledTimes(3);
    } finally {
      if (wrapper.exists()) wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it("reviewer: item rows do not display source-code comments", async () => {
    getTaskItems.mockResolvedValue({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();
    const text = wrapper.text();
    wrapper.unmount();
    expect(text).not.toContain("// Reuses the shared per-item action");
  });

  it("reviewer: periodic refresh reloads the loaded pages and their cursor", async () => {
    vi.useFakeTimers();
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never)
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "refresh-cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    try {
      await flushPromises();
      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("item-2");
      await vi.advanceTimersByTimeAsync(3000);
      await flushPromises();
      expect(wrapper.text()).toContain("item-2");
      expect(wrapper.find('[data-testid="task-items-load-more"]').exists()).toBe(false);
      expect(getTaskItems).toHaveBeenCalledTimes(4);
      expect(getTaskItems).toHaveBeenNthCalledWith(3, "task-id", expect.objectContaining({ limit: 100 }));
      expect(getTaskItems).toHaveBeenNthCalledWith(
        4,
        "task-id",
        expect.objectContaining({ limit: 100, cursor: "refresh-cursor-1" }),
      );
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it("periodic refresh preserves the active status filter across loaded pages", async () => {
    vi.useFakeTimers();
    getTaskItems
      .mockResolvedValueOnce({ items: [item("all-1")], next_cursor: "all-cursor" } as never)
      .mockResolvedValueOnce({ items: [item("failed-1", "failed")], next_cursor: "failed-cursor" } as never)
      .mockResolvedValueOnce({ items: [item("failed-2", "failed")], next_cursor: null } as never)
      .mockResolvedValueOnce({ items: [item("failed-1", "failed")], next_cursor: "refresh-failed-cursor" } as never)
      .mockResolvedValueOnce({ items: [item("failed-2", "failed")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    try {
      await flushPromises();
      const trigger = wrapper.findComponent('[data-testid="task-items-filter"]') as unknown as {
        vm: { $parent: unknown };
      };
      let selectVm = trigger.vm.$parent as unknown as {
        $options?: { __name?: string };
        $parent?: unknown;
        $emit?: (event: string, value: unknown) => void;
      } | null;
      while (selectVm && selectVm.$options?.__name !== "Select") {
        selectVm = selectVm.$parent as typeof selectVm;
      }
      selectVm!.$emit!("update:model-value", "failed");
      await flushPromises();
      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();

      await vi.advanceTimersByTimeAsync(3000);
      await flushPromises();

      expect(getTaskItems).toHaveBeenCalledTimes(5);
      for (const call of getTaskItems.mock.calls.slice(1)) {
        expect(call[1]).toEqual(expect.objectContaining({ status: "failed" }));
      }
      expect(getTaskItems).toHaveBeenLastCalledWith(
        "task-id",
        expect.objectContaining({ status: "failed", cursor: "refresh-failed-cursor" }),
      );
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it("reviewer: a failed refresh page preserves loaded rows and the prior cursor", async () => {
    vi.useFakeTimers();
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: "cursor-2" } as never)
      .mockResolvedValueOnce({ items: [item("new-item-1")], next_cursor: "new-cursor-1" } as never)
      .mockRejectedValueOnce(new Error("later page failed"))
      .mockResolvedValueOnce({ items: [item("item-3")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    try {
      await flushPromises();
      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();
      expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(2);

      await vi.advanceTimersByTimeAsync(3000);
      await flushPromises();
      expect(wrapper.find('[data-testid="task-items-refresh-failed"]').exists()).toBe(true);
      expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(2);
      expect(wrapper.text()).not.toContain("new-item-1");

      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();
      expect(getTaskItems).toHaveBeenLastCalledWith(
        "task-id",
        expect.objectContaining({ limit: 100, cursor: "cursor-2" }),
      );
      expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(3);
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it("settles load-more state after failure so the same cursor can be retried", async () => {
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockRejectedValueOnce(new Error("page failed"))
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
    await flushPromises();
    expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(1);

    await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
    await flushPromises();
    expect(getTaskItems).toHaveBeenCalledTimes(3);
    expect(getTaskItems).toHaveBeenLastCalledWith(
      "task-id",
      expect.objectContaining({ cursor: "cursor-1" }),
    );
    expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(2);
    wrapper.unmount();
  });

  it("does not overlap a load-more request with a periodic refresh", async () => {
    vi.useFakeTimers();
    let resolveMore!: (value: Awaited<ReturnType<typeof apiClient.getTaskItems>>) => void;
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveMore = resolve; }))
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "refresh-cursor" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    try {
      await flushPromises();
      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();
      await vi.advanceTimersByTimeAsync(3000);
      expect(getTaskItems).toHaveBeenCalledTimes(2);

      resolveMore({ items: [item("item-2")], next_cursor: null } as never);
      await flushPromises();
      expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(2);
      await vi.advanceTimersByTimeAsync(3000);
      await flushPromises();
      expect(getTaskItems).toHaveBeenCalledTimes(4);
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it("reviewer: a timer does not overlap an initial load or strand its spinner", async () => {
    vi.useFakeTimers();
    let resolveInitial!: (value: Awaited<ReturnType<typeof apiClient.getTaskItems>>) => void;
    getTaskItems
      .mockImplementationOnce(() => new Promise((resolve) => { resolveInitial = resolve; }))
      .mockResolvedValue({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    try {
      await vi.advanceTimersByTimeAsync(3000);
      await flushPromises();
      resolveInitial({ items: [item("item-1")], next_cursor: null } as never);
      await flushPromises();
      expect(wrapper.find('[data-testid="task-item-row"]').exists()).toBe(true);
      expect(getTaskItems).toHaveBeenCalledTimes(1);
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it("loads the first page and shows the shown/total count", async () => {
    getTaskItems.mockResolvedValue({ items: [item("item-1")], next_cursor: "cursor-1" } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(1);
    expect(getTaskItems).toHaveBeenCalledWith("task-id", expect.objectContaining({ limit: 100 }));
    expect(wrapper.find('[data-testid="task-items-count"]').text()).toContain("1");
    expect(wrapper.find('[data-testid="task-items-count"]').text()).toContain("2781");
    expect(wrapper.text()).toContain("item-1");
    expect(wrapper.find('[data-testid="task-items-load-more"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("follows next_cursor on load more until the cursor is null", async () => {
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(getTaskItems).toHaveBeenLastCalledWith(
      "task-id",
      expect.objectContaining({ limit: 100, cursor: "cursor-1" }),
    );
    expect(wrapper.text()).toContain("item-1");
    expect(wrapper.text()).toContain("item-2");
    expect(wrapper.find('[data-testid="task-items-count"]').text()).toContain("2");
    expect(wrapper.find('[data-testid="task-items-load-more"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("resets the cursor and sends the status filter when the filter changes", async () => {
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("failed-1", "failed")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    setItemStatusFilter(wrapper, "failed");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    // Cursor is scoped to the filter: a filter change restarts without one.
    expect(getTaskItems).toHaveBeenLastCalledWith(
      "task-id",
      expect.objectContaining({ limit: 100, status: "failed" }),
    );
    expect(wrapper.text()).toContain("failed-1");
    expect(wrapper.text()).not.toContain("item-1");
    wrapper.unmount();
  });

  it("discards an old filter response and serializes the new filter request", async () => {
    let resolveFailed!: (value: Awaited<ReturnType<typeof apiClient.getTaskItems>>) => void;
    getTaskItems
      .mockResolvedValueOnce({ items: [item("all-item")], next_cursor: null } as never)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFailed = resolve; }))
      .mockResolvedValueOnce({
        items: [{ ...item("current-item", "succeeded"), file_name: "current-success.pdf" }],
        next_cursor: null,
      } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    setItemStatusFilter(wrapper, "failed");
    await flushPromises();
    expect(getTaskItems).toHaveBeenCalledTimes(2);

    setItemStatusFilter(wrapper, "succeeded");
    await flushPromises();
    expect(getTaskItems).toHaveBeenCalledTimes(2);

    resolveFailed({
      items: [{ ...item("stale-item", "failed"), file_name: "stale-failure.pdf" }],
      next_cursor: null,
    } as never);
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(3);
    expect(getTaskItems).toHaveBeenLastCalledWith(
      "task-id",
      expect.objectContaining({ status: "succeeded" }),
    );
    expect(wrapper.text()).toContain("current-success.pdf");
    expect(wrapper.text()).not.toContain("stale-failure.pdf");
    wrapper.unmount();
  });

  it("exposes a retry-load button when the first page fails and reuses getTaskItems", async () => {    getTaskItems
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.text()).toContain("Failed to load task items");
    const retryLoad = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Retry loading items");
    expect(retryLoad).toBeDefined();
    await retryLoad!.trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("item-1");
    wrapper.unmount();
  });

  it("keeps the full item error text in the tooltip instead of truncating at 240", async () => {
    const longError = `Qdrant unavailable: ${"x".repeat(400)}`;
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1", "failed"), error_message: longError }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(longError.length).toBeGreaterThan(240);
    expect(wrapper.text()).toContain(longError);
    const errorNode = wrapper.find('[data-testid="task-item-error"]');
    expect(errorNode.attributes("title")).toBe(longError);
    wrapper.unmount();
  });

  it("renders collapsed stages 1:1 without a Docling converting label", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        { ...item("collapsed-1", "running"), stage: "processing" },
        { ...item("collapsed-2", "running"), stage: "finalize" },
      ],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    // Items without a progress record fall back to their persisted stage, so
    // the stage vocabulary stays readable exactly as before.
    const stages = wrapper.findAll('[data-testid="task-item-stage"]').map((node) => node.text());
    expect(stages).toEqual(["Processing", "Finalize"]);
    expect(wrapper.text()).not.toContain("Converting remotely");
    wrapper.unmount();
  });

  it("does not render an external-job badge column", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1", "running") }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.text()).not.toContain("Quarantined");
    expect(wrapper.text()).not.toContain("Docling queued");
    wrapper.unmount();
  });

  it("renders entry stages with localized labels instead of raw i18n keys", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        { ...item("download-1", "running"), stage: "download" },
        { ...item("storage-1", "running"), stage: "storage" },
        { ...item("sync-1", "running"), stage: "sync" },
        { ...item("delete-1", "running"), stage: "delete" },
        { ...item("translation-1", "running"), stage: "translation" },
        { ...item("indexing-1", "running"), stage: "indexing" },
      ],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const rendered = wrapper.findAll('[data-testid="task-item-stage"]').map((node) => node.text());
    expect(rendered).toEqual(["Download", "Storage", "Sync", "Delete", "Translation", "Indexing"]);
    expect(wrapper.text()).not.toContain("processingQueue.stages.");
    wrapper.unmount();
  });

  // Identity and attempt bookkeeping remain available behind the disclosure;
// what must not return is the panel-wide "Task details" list that repeated the
  // collapsed row's type, group and timestamps verbatim.
  it("moves the task UUID and the item identifiers into a structured detail panel", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        {
          ...item("item-uuid-1", "failed"),
          ordinal: 3,
          file_name: "2026-09-30-report.pdf",
          document_title: "Q3 disclosure report",
          error_message: "Qdrant unreachable",
        },
      ],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-detail-panel"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("task-id");
    expect(wrapper.text()).not.toContain("Task details");

    const detail = wrapper.find('[data-testid="task-item-detail"]');
    expect(detail.exists()).toBe(true);
    for (const label of ["Item ID", "Position", "Stage", "Attempts"]) {
      expect(detail.text()).toContain(label);
    }
    expect(detail.text()).toContain("item-uuid-1");
    // The readable name stays primary so two items remain distinguishable.
    expect(wrapper.find('[data-testid="task-item-subject"]').text()).toBe("2026-09-30-report.pdf");
    expect(wrapper.find('[data-testid="task-item-row"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("omits the file-name, title, and dependency rows for an item that has none", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1", "queued") }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const detail = wrapper.find('[data-testid="task-item-detail"]');
    expect(detail.exists()).toBe(true);
    for (const label of ["Dependency", "File name", "Document title", "File ID"]) {
      expect(detail.text()).not.toContain(label);
    }
    wrapper.unmount();
  });

  it("falls back to Unknown for unknown and null item stages", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        { ...item("unknown-1", "running"), stage: "future_stage" },
        { ...item("null-1", "running"), stage: null },
      ],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    // An unrecognized stage is labelled Unknown rather than leaking the raw
    // identifier or a raw `processingQueue.stages.*` key.
    expect(wrapper.text()).not.toContain("future_stage");
    expect(wrapper.text()).not.toContain("processingQueue.stages.");
    expect(wrapper.find('[data-testid="task-item-detail"]').text()).toContain("Unknown");
    wrapper.unmount();
  });

  it("renders the expanded row status badge through locale keys, not raw identifiers", async () => {
    getTaskItems.mockResolvedValue({
      items: [item("row-1", "failed"), item("row-2", "waiting")],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded(task(), "zh-CN");
    await flushPromises();

    const badges = wrapper.findAll('[data-testid="task-item-status"]').map((node) => node.text());
    expect(badges).toEqual(["失败", "等待中"]);
    wrapper.unmount();
  });

  it("mounts the diagnose panel that fetches only after an explicit toggle with the rendered task id", async () => {
    getTaskItems.mockResolvedValue({ items: [], next_cursor: null } as never);
    getTaskDiagnose.mockResolvedValue({
      task: {
        task_id: "rendered-task-id",
        kind: "file_batch",
        status: "waiting",
        progress: { total: 0, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
        stage: "processing",
        waiting_reason: null,
        dependency_key: null,
        next_attempt_at: null,
        failure_stage: null,
        error_summary: null,
        lease_expires_at: null,
        created_at: "2026-07-20T00:01:00Z",
        started_at: null,
        finished_at: null,
        updated_at: "2026-07-20T00:02:00Z",
      },
      items: [],
      items_truncated: false,
      dependency_gates: [],
      consistency: {
        consistent: true,
        current_item_id: null,
        mismatches: [],
        open_attempt_count: 0,
        near_exhaustion_item_count: 0,
      },
      observed_at: "2026-07-20T00:02:00Z",
    } as never);
    const wrapper = mountExpanded({ ...task(), task_id: "rendered-task-id" });
    await flushPromises();

    const toggle = wrapper.find('[data-testid="task-diagnose-toggle"]');
    expect(toggle.exists()).toBe(true);
    expect(getTaskDiagnose).not.toHaveBeenCalled();

    await toggle.trigger("click");
    await flushPromises();

    expect(getTaskDiagnose).toHaveBeenCalledTimes(1);
    expect(getTaskDiagnose).toHaveBeenCalledWith("rendered-task-id");
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("rebinds the diagnose panel to a new task id and discards the previous task response", async () => {
    getTaskItems.mockResolvedValue({ items: [], next_cursor: null } as never);
    let resolveFirst: ((value: TaskDiagnoseResponse) => void) | null = null;
    getTaskDiagnose
      .mockImplementationOnce(
        () => new Promise<TaskDiagnoseResponse>((resolve) => { resolveFirst = resolve; }),
      )
      .mockResolvedValueOnce(diagnoseFor("task-b") as never);

    const wrapper = mountExpanded({ ...task(), task_id: "task-a" });
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();
    expect(getTaskDiagnose).toHaveBeenLastCalledWith("task-a");

    await wrapper.setProps({ task: { ...task(), task_id: "task-b" } });
    await flushPromises();

    resolveFirst!(diagnoseFor("task-a") as never);
    await flushPromises();
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="task-diagnose-error"]').exists()).toBe(false);
    expect(getTaskDiagnose).toHaveBeenCalledTimes(1);
    expect(getTaskDiagnose).not.toHaveBeenCalledWith("task-b");
    wrapper.unmount();
  });

  // The panel used to open with a full task definition list that repeated the
  // collapsed row's type, group and timestamps verbatim, and a per-item list
  // that repeated the same identity a third time. Progress is now the primary
  // content and the metadata is not restated.
  it("leads with item progress instead of a duplicated task metadata panel", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1"), progress: indexingProgress({ completed_chunks: 70, total_chunks: 512, completed_batches: 3, total_batches: 16 }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-detail-panel"]').exists()).toBe(false);
    const progress = wrapper.find('[data-testid="task-item-progress"]');
    expect(progress.exists()).toBe(true);
    expect(progress.text()).toContain("70 / 512 chunks");
    expect(progress.text()).toContain("3 / 16 batches");
    expect(progress.text()).toContain("Indexing");
    // The task's own identifiers and lifecycle timestamps are the row's job.
    expect(wrapper.text()).not.toContain("Task ID");
    wrapper.unmount();
  });

  it("shows no percentage when a counter total is unknown", async () => {
    getTaskItems.mockResolvedValue({
      // A resume publishes planned totals before it has re-counted its skipped
      // chunks; there is no denominator to build a fraction from.
      items: [{ ...item("item-1"), progress: indexingProgress({ total_chunks: 512, total_batches: 16 }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const progress = wrapper.find('[data-testid="task-item-progress"]');
    expect(progress.find('[data-testid="item-progress-bar"]').exists()).toBe(false);
    expect(progress.find('[data-testid="item-progress-unknown"]').exists()).toBe(true);
    expect(progress.text()).not.toContain("%");
    wrapper.unmount();
  });

  it("renders a bar only for a known total greater than zero", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1"), progress: indexingProgress({ completed_chunks: 32, total_chunks: 512, completed_batches: 1, total_batches: 16 }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.findAll('[data-testid="item-progress-bar"]').length).toBe(2);
    wrapper.unmount();
  });

  it("marks an empty document complete instead of rendering an indeterminate bar", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1", "succeeded"), progress: indexingProgress({ stage: "chunking", completed_chunks: 0, total_chunks: 0, completed_batches: 0, total_batches: 0 }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.find('[data-testid="item-progress-complete"]').exists()).toBe(true);
    expect(wrapper.findAll('[data-testid="item-progress-bar"]').length).toBe(0);
    wrapper.unmount();
  });

  it("keeps chunk and batch counters as separate units", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1"), progress: indexingProgress({ completed_chunks: 70, total_chunks: 512, completed_batches: 3, total_batches: 16 }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const chunks = wrapper.find('[data-testid="item-progress-chunks"]').text();
    const batches = wrapper.find('[data-testid="item-progress-batches"]').text();
    expect(chunks).toContain("70 / 512 chunks");
    expect(batches).toContain("3 / 16 batches");
    expect(chunks).not.toContain("batch");
    wrapper.unmount();
  });

  it("says page progress is unavailable for Docling but never for text ingestion", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        { ...item("docling-1"), progress: indexingProgress({ stage: "docling", remote_status: "started" }) },
        { ...item("text-1"), progress: indexingProgress({ stage: "parsing" }) },
      ],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const unavailable = wrapper.find('[data-testid="item-progress-page-unavailable"]');
    expect(unavailable.exists()).toBe(true);
    expect(unavailable.text()).toContain("does not report page progress");
    expect(wrapper.find('[data-testid="item-progress-remote-status"]').text()).toContain("started");
    // Text ingestion has no remote conversion and must not imply one.
    expect(wrapper.findAll('[data-testid="item-progress-page-unavailable"]')).toHaveLength(1);
    wrapper.unmount();
  });

  it("shows the last observed progress time as an observation, not a stall claim", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1"), progress: indexingProgress({ total_chunks: 512, last_progress_at: "2026-07-20T00:02:00Z" }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.find('[data-testid="item-progress-observed"]').text()).toContain("Last progress");
    expect(wrapper.text()).not.toContain("stalled");
    wrapper.unmount();
  });

  it("uses two columns on a wide panel and stacks them on a narrow one", async () => {
    getTaskItems.mockResolvedValue({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    // A single responsive grid carries both halves: two columns at `lg` and
    // above, one stacked column below it.
    const grid = wrapper.find('[data-testid="task-item-row"] > div');
    expect(grid.classes().join(" ")).toContain("grid");
    // `lg:` prefix is what makes the layout two columns on wide screens and a
    // single stacked column on narrow ones.
    expect(grid.classes().join(" ")).toContain("lg:grid-cols-");
    wrapper.unmount();
  });

  it("keeps item identity visible so a multi-item task stays distinguishable", async () => {
    getTaskItems.mockResolvedValue({
      items: [
        { ...item("item-1"), ordinal: 0, file_name: "report.pdf" },
        { ...item("item-2"), ordinal: 1, file_name: "summary.docx" },
      ],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const subjects = wrapper.findAll('[data-testid="task-item-subject"]').map((node) => node.text());
    expect(subjects).toEqual(["report.pdf", "summary.docx"]);
    wrapper.unmount();
  });

  it("falls back to the item position when an item has no readable name", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1"), ordinal: 4 }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-item-subject"]').text()).toBe("Item 4");
    wrapper.unmount();
  });

  it("keeps the failure reason visible and moves identifiers behind a disclosure", async () => {
    getTaskItems.mockResolvedValue({
      items: [{
        ...item("item-1", "failed"),
        retryable: true,
        error_message: "Qdrant unreachable",
      }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.find('[data-testid="task-item-error"]').text()).toBe("Qdrant unreachable");
    // The retry label states the real scope: the endpoint requeues the whole
    // task's retryable items, not just the clicked one.
    expect(wrapper.find('[data-testid="task-item-retry"]').text()).toBe("Retry this task");
    expect(wrapper.find('[data-testid="task-item-technical-toggle"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("makes diagnosis a secondary, explained disclosure below the items", async () => {
    getTaskItems.mockResolvedValue({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const disclosure = wrapper.find('[data-testid="task-diagnose-disclosure"]');
    expect(disclosure.exists()).toBe(true);
    const explanation = wrapper.find('[data-testid="task-diagnose-explanation"]');
    expect(explanation.text()).toContain("Read-only troubleshooting");
    expect(explanation.text()).toContain("does not change this task");
    expect(getTaskDiagnose).not.toHaveBeenCalled();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();
    expect(getTaskDiagnose).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("does not repeat the collapsed row's metadata as a second definition list", async () => {
    getTaskItems.mockResolvedValue({
      items: [{ ...item("item-1"), progress: indexingProgress({ total_chunks: 4 }) }],
      next_cursor: null,
    } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    // Identity is still inspectable, but only through the disclosure.
    expect(wrapper.find('[data-testid="task-item-detail"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("Task details");
    wrapper.unmount();
  });

  // The parent's `updated_at` moves only when the task is claimed and when it
  // recomputes, so it stays constant through a long indexing run. Without a
  // timer of its own the panel would never show that progress advanced.
  it("reloads on its own cadence even though the task timestamp never changes", async () => {
    const { ITEM_REFRESH_INTERVAL_MS } = await import("../composables/use-task-items-refresh");
    vi.useFakeTimers();
    getTaskItems
      .mockResolvedValueOnce({ items: [{ ...item("item-1"), progress: indexingProgress({ total_chunks: 512 }) }], next_cursor: null } as never)
      .mockResolvedValue({ items: [{ ...item("item-1"), progress: indexingProgress({ completed_chunks: 512, total_chunks: 512 }) }], next_cursor: null } as never);
    const entry = task();
    const wrapper = mountExpanded(entry);
    await flushPromises();

    // The task object is deliberately the same one throughout: only
    // `updated_at` matters, and it is untouched.
    expect(getTaskItems).toHaveBeenCalledTimes(1);
    expect(wrapper.find('[data-testid="item-progress-complete"]').exists()).toBe(false);

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(wrapper.find('[data-testid="item-progress-complete"]').exists()).toBe(true);
    wrapper.unmount();
    vi.useRealTimers();
  });

  it("keeps the last valid items and shows a non-destructive notice when a refresh fails", async () => {
    const { ITEM_REFRESH_INTERVAL_MS } = await import("../composables/use-task-items-refresh");
    vi.useFakeTimers();
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: null } as never)
      .mockRejectedValue(new Error("network down"));
    const wrapper = mountExpanded();
    await flushPromises();

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    await flushPromises();

    const notice = wrapper.find('[data-testid="task-items-refresh-failed"]');
    expect(notice.exists()).toBe(true);
    expect(notice.text()).toContain("showing the last known progress");
    // The previously loaded item survives the failure.
    expect(wrapper.find('[data-testid="task-item-row"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("item-1");
    wrapper.unmount();
    vi.useRealTimers();
  });

  it("stops reloading after unmount so a collapsed panel does not keep polling", async () => {
    const { ITEM_REFRESH_INTERVAL_MS } = await import("../composables/use-task-items-refresh");
    vi.useFakeTimers();
    getTaskItems.mockResolvedValue({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();
    const callsAfterMount = getTaskItems.mock.calls.length;

    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS * 3);

    expect(getTaskItems.mock.calls.length).toBe(callsAfterMount);
    vi.useRealTimers();
  });

  it("does not reload a terminal task", async () => {
    const { ITEM_REFRESH_INTERVAL_MS } = await import("../composables/use-task-items-refresh");
    vi.useFakeTimers();
    getTaskItems.mockResolvedValue({ items: [item("item-1", "succeeded")], next_cursor: null } as never);
    const wrapper = mountExpanded({ ...task(), status: "succeeded" });
    await flushPromises();
    const callsAfterMount = getTaskItems.mock.calls.length;

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS * 3);

    expect(getTaskItems.mock.calls.length).toBe(callsAfterMount);
    wrapper.unmount();
    vi.useRealTimers();
  });

  it("discards an in-flight response that belongs to a task the panel no longer shows", async () => {
    const { ITEM_REFRESH_INTERVAL_MS } = await import("../composables/use-task-items-refresh");
    vi.useFakeTimers();
    // Typed as the resolved shape so the late answer satisfies the spy; the point
    // of the test is that it is discarded, not that it type-checks as `unknown`.
    let resolveFirst: ((value: Awaited<ReturnType<typeof apiClient.getTaskItems>>) => void) | null = null;
    getTaskItems
      .mockImplementationOnce(
        () => new Promise<Awaited<ReturnType<typeof apiClient.getTaskItems>>>((resolve) => {
          resolveFirst = resolve;
        }),
      )
      .mockResolvedValue({
        items: [{ ...item("task-b-item"), file_name: "task-b-document.pdf" }],
        next_cursor: null,
      } as never);
    const wrapper = mountExpanded({ ...task(), task_id: "task-a" });
    await flushPromises();

    await wrapper.setProps({ task: { ...task(), task_id: "task-b" } });
    resolveFirst!({ items: [item("stale-item")], next_cursor: null } as never);
    await flushPromises();
    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("task-b-document.pdf");
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    await flushPromises();

    // The late answer for task-a must never be painted under task-b.
    expect(wrapper.text()).not.toContain("stale-item");
    wrapper.unmount();
    vi.useRealTimers();
  });
});
