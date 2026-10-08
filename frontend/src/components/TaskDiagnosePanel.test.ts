import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import TaskDiagnosePanel from "./TaskDiagnosePanel.vue";
import { apiClient, type TaskDiagnoseItem, type TaskDiagnoseResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const getTaskDiagnose = vi.spyOn(apiClient, "getTaskDiagnose");

function diagnoseItem(ordinal: number): TaskDiagnoseItem {
  return {
    item_id: `diag-item-${ordinal}`,
    ordinal,
    status: "waiting",
    stage: "processing",
    waiting_reason: "dependency",
    dependency_key: "s3",
    next_attempt_at: "2026-07-20T00:10:00Z",
    failure_stage: null,
    error_message: ordinal === 1 ? "Qdrant unavailable" : null,
    attempt_count: 3,
    retryable: true,
    lease_expires_at: null,
    active_attempt: null,
    latest_attempt: null,
    created_at: "2026-07-20T00:01:00Z",
    started_at: "2026-07-20T00:01:00Z",
    finished_at: null,
  };
}

function gate(state: string, dependencyKey = "s3"): TaskDiagnoseResponse["dependency_gates"][number] {
  return {
    dependency_key: dependencyKey,
    failure_count: 2,
    last_error: "timeout",
    last_success_at: null,
    last_transition_at: "2026-07-20T00:02:00Z",
    next_probe_at: "2026-07-20T00:03:00Z",
    state,
  };
}

function diagnoseResponse(overrides: Partial<TaskDiagnoseResponse> = {}): TaskDiagnoseResponse {
  return {
    task: {
      task_id: "task-id",
      kind: "file_batch",
      status: "waiting",
      progress: { total: 2, queued: 0, running: 0, waiting: 2, succeeded: 0, failed: 0, cancelled: 0 },
      stage: "processing",
      waiting_reason: "dependency",
      dependency_key: "s3",
      next_attempt_at: "2026-07-20T00:10:00Z",
      failure_stage: null,
      error_summary: null,
      lease_expires_at: "2026-07-20T00:05:00Z",
      created_at: "2026-07-20T00:01:00Z",
      started_at: "2026-07-20T00:01:00Z",
      finished_at: null,
      updated_at: "2026-07-20T00:02:00Z",
    },
    items: [diagnoseItem(0), diagnoseItem(1)],
    items_truncated: false,
    dependency_gates: [],
    consistency: {
      consistent: true,
      current_item_id: "diag-item-0",
      mismatches: [],
      open_attempt_count: 0,
      near_exhaustion_item_count: 0,
    },
    observed_at: "2026-07-20T00:02:00Z",
    ...overrides,
  };
}

function mountPanel(taskId = "task-id", locale: "en" | "zh-CN" = "en") {
  return mount(TaskDiagnosePanel, {
    props: { taskId },
    global: { plugins: [testNuxtUiPlugin, createTestI18n(locale)] },
  });
}

describe("TaskDiagnosePanel", () => {
  beforeEach(() => {
    getTaskDiagnose.mockReset();
  });

  it("does not fetch the diagnose projection until the toggle is used", async () => {
    const wrapper = mountPanel();
    await flushPromises();

    expect(getTaskDiagnose).not.toHaveBeenCalled();
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("loads the diagnose projection on demand and renders its item detail", async () => {
    getTaskDiagnose.mockResolvedValue(diagnoseResponse() as never);
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    expect(getTaskDiagnose).toHaveBeenCalledTimes(1);
    expect(getTaskDiagnose).toHaveBeenCalledWith("task-id");
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="task-diagnose-consistency"]').text()).toContain("agree");

    const facts = wrapper.findAll('[data-testid="diagnose-facts"]');
    expect(facts).toHaveLength(2);
    expect(facts[0]!.text()).toContain("Position 1");
    expect(facts[0]!.text()).toContain("ordinal 0");
    expect(facts[0]!.text()).toContain("Processing");
    expect(facts[0]!.text()).toContain("3 attempts");
    expect(facts[0]!.text()).toContain("Dependency: S3");
    expect(facts[0]!.text()).toContain("Next retry");
    expect(facts[1]!.text()).toContain("Position 2");
    expect(facts[1]!.text()).toContain("Latest error: Qdrant unavailable");
    wrapper.unmount();
  });

  it("sorts diagnose items by ordinal instead of trusting response order", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({ items: [diagnoseItem(2), diagnoseItem(0), diagnoseItem(1)] }) as never,
    );
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    const positions = wrapper.findAll('[data-testid="diagnose-facts"]').map((node) => node.text());
    expect(positions[0]).toContain("Position 1");
    expect(positions[1]).toContain("Position 2");
    expect(positions[2]).toContain("Position 3");
    wrapper.unmount();
  });

  it("surfaces a consistency warning, mismatch fields, and truncation", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({
        items_truncated: true,
        consistency: {
          consistent: false,
          current_item_id: null,
          mismatches: ["running_count", "stage"],
          open_attempt_count: 2,
          near_exhaustion_item_count: 1,
        },
      }) as never,
    );
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    expect(wrapper.find('[data-testid="task-diagnose-consistency"]').text()).toContain("disagree");
    expect(wrapper.find('[data-testid="task-diagnose-mismatches"]').text()).toBe("Mismatched fields: running_count, stage");
    expect(wrapper.find('[data-testid="task-diagnose-truncated"]').text()).toContain("first 2 items");
    expect(wrapper.text()).toContain("2 open attempts");
    expect(wrapper.text()).toContain("1 items near the retry limit");
    wrapper.unmount();
  });

  it("toggles the diagnose panel closed and back off without refetching", async () => {
    getTaskDiagnose.mockResolvedValue(diagnoseResponse() as never);
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();
    expect(getTaskDiagnose).toHaveBeenCalledTimes(1);

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(false);
    expect(getTaskDiagnose).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("renders a retry affordance when the diagnose projection fails", async () => {
    getTaskDiagnose
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce(diagnoseResponse() as never);
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    expect(wrapper.find('[data-testid="task-diagnose-error"]').text()).toContain("Failed to load diagnose");
    const retry = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Retry diagnose");
    expect(retry).toBeDefined();
    await retry!.trigger("click");
    await flushPromises();

    expect(getTaskDiagnose).toHaveBeenCalledTimes(2);
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("renders lease, active-attempt, and dependency facts from the diagnose projection", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({
        items: [
          {
            ...diagnoseItem(0),
            waiting_reason: null,
            dependency_key: "qdrant",
            lease_expires_at: "2026-07-20T00:05:00Z",
            active_attempt: {
              attempt: 4,
              attempt_id: 4,
              error_message: null,
              failure_stage: null,
              finished_at: null,
              retryable: true,
              started_at: "2026-07-20T00:01:00Z",
              status: "running",
            },
          },
        ],
      }) as never,
    );
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    const facts = wrapper.find('[data-testid="diagnose-facts"]');
    expect(facts.text()).toContain("Dependency: Qdrant");
    expect(facts.text()).toContain("Lease until");
    expect(facts.text()).toContain("Active attempt 4");
    wrapper.unmount();
  });

  it("discards an in-flight diagnose response after the task id changes", async () => {
    let resolveDiagnose: ((value: TaskDiagnoseResponse) => void) | null = null;
    getTaskDiagnose.mockImplementationOnce(
      () => new Promise<TaskDiagnoseResponse>((resolve) => { resolveDiagnose = resolve; }),
    );
    const wrapper = mountPanel("task-1");
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    await wrapper.setProps({ taskId: "task-2" });
    await flushPromises();

    resolveDiagnose!(diagnoseResponse() as never);
    await flushPromises();

    expect(getTaskDiagnose).toHaveBeenCalledWith("task-1");
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="task-diagnose-error"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("localizes dependency-gate states instead of exposing backend identifiers", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({
        dependency_gates: [gate("half_open"), gate("closed", "qdrant")],
      }) as never,
    );
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    const gates = wrapper.find('[data-testid="task-diagnose-gates"]');
    expect(gates.exists()).toBe(true);
    expect(gates.text()).toContain("Half-open");
    expect(gates.text()).toContain("Closed");
    expect(gates.text()).not.toContain("half_open");
    expect(gates.text()).not.toContain("state:");
    wrapper.unmount();
  });

  it("renders Chinese diagnose labels for item status, gate state, and reasoning", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({ dependency_gates: [gate("open"), gate("half_open", "qdrant")] }) as never,
    );
    const wrapper = mountPanel("task-id", "zh-CN");
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    expect(wrapper.findAll('[data-testid="diagnose-status"]')[0]!.text()).toBe("等待中");
    const gates = wrapper.find('[data-testid="task-diagnose-gates"]');
    expect(gates.text()).toContain("已开启");
    expect(gates.text()).toContain("半开");
    expect(wrapper.text()).toContain("依赖不可用: S3");
    expect(wrapper.text()).not.toContain("waiting");
    expect(wrapper.text()).not.toContain("half_open");
    expect(wrapper.text()).not.toContain("open");
    wrapper.unmount();
  });

  it("discards a stale diagnose failure after the task id changes", async () => {
    let rejectDiagnose: ((reason: Error) => void) | null = null;
    getTaskDiagnose.mockImplementationOnce(
      () => new Promise<TaskDiagnoseResponse>((_resolve, reject) => { rejectDiagnose = reject; }),
    );
    const wrapper = mountPanel("task-1");
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    await wrapper.setProps({ taskId: "task-2" });
    await flushPromises();

    rejectDiagnose!(new Error("network down"));
    await flushPromises();

    expect(wrapper.find('[data-testid="task-diagnose-error"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="task-diagnose-panel"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("localizes every known gate state in zh-CN and leaks no raw identifier", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({
        dependency_gates: [gate("open"), gate("half_open", "qdrant"), gate("closed", "embedding")],
      }) as never,
    );
    const wrapper = mountPanel("task-id", "zh-CN");
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    const gates = wrapper.find('[data-testid="task-diagnose-gates"]');
    expect(gates.text()).toContain("已开启");
    expect(gates.text()).toContain("半开");
    expect(gates.text()).toContain("已关闭");
    expect(gates.text()).not.toContain("open");
    expect(gates.text()).not.toContain("half_open");
    expect(gates.text()).not.toContain("closed");
    wrapper.unmount();
  });

  it("falls back to the raw gate state for a value this build does not know", async () => {
    getTaskDiagnose.mockResolvedValue(
      diagnoseResponse({ dependency_gates: [gate("future_state")] }) as never,
    );
    const wrapper = mountPanel();
    await flushPromises();

    await wrapper.find('[data-testid="task-diagnose-toggle"]').trigger("click");
    await flushPromises();

    const gates = wrapper.find('[data-testid="task-diagnose-gates"]');
    expect(gates.exists()).toBe(true);
    expect(gates.text()).toContain("future_state");
    wrapper.unmount();
  });
});
