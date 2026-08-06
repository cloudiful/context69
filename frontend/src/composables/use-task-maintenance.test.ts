import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskMaintenanceOverview } from "../services/api";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { useTaskMaintenance } from "./use-task-maintenance";

const getTaskMaintenance = vi.spyOn(apiClient, "getTaskMaintenance");
const updateTaskMaintenance = vi.spyOn(apiClient, "updateTaskMaintenance");
const cancelActiveTasks = vi.spyOn(apiClient, "cancelActiveTasks");
const purgeTasks = vi.spyOn(apiClient, "purgeTasks");

const overview: TaskMaintenanceOverview = {
  settings: {
    cleanup_enabled: true,
    retention_days: 30,
    updated_at: "2026-07-20T00:00:00Z",
  },
  stats: {
    total: 40,
    queued: 2,
    running: 1,
    waiting: 3,
    succeeded: 25,
    failed: 5,
    cancelled: 4,
    active: 6,
    expired_terminal: 12,
  },
};

describe("useTaskMaintenance", () => {
  beforeEach(() => {
    setAuthenticatedUser({ is_admin: true });
    getTaskMaintenance.mockReset().mockResolvedValue(overview as never);
    updateTaskMaintenance.mockReset().mockResolvedValue(overview as never);
    cancelActiveTasks.mockReset().mockResolvedValue({ cancelled_tasks: 6 } as never);
    purgeTasks.mockReset().mockResolvedValue({ deleted_tasks: 12 } as never);
  });

  function mountState() {
    let state!: ReturnType<typeof useTaskMaintenance>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useTaskMaintenance({ t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
    return { state, wrapper };
  }

  it("loads the overview for admins", async () => {
    const { state, wrapper } = mountState();
    await state.load();
    expect(getTaskMaintenance).toHaveBeenCalledOnce();
    expect(state.settings.value?.retention_days).toBe(30);
    expect(state.stats.value?.active).toBe(6);
    expect(state.activeCount.value).toBe(6);
    wrapper.unmount();
  });

  it("skips loading for non-admin users", async () => {
    setGuest();
    const { state, wrapper } = mountState();
    await state.load();
    expect(getTaskMaintenance).not.toHaveBeenCalled();
    expect(state.isAdmin.value).toBe(false);
    wrapper.unmount();
  });

  it("saves settings and updates the overview", async () => {
    updateTaskMaintenance.mockResolvedValue({
      ...overview,
      settings: { ...overview.settings, cleanup_enabled: false, retention_days: 7 },
    } as never);
    const { state, wrapper } = mountState();
    await state.load();
    await state.saveSettings(false, 7);
    expect(updateTaskMaintenance).toHaveBeenCalledWith({ cleanup_enabled: false, retention_days: 7 });
    expect(state.settings.value?.cleanup_enabled).toBe(false);
    expect(state.settings.value?.retention_days).toBe(7);
    wrapper.unmount();
  });

  it("cancels all active tasks and refreshes the overview", async () => {
    const { state, wrapper } = mountState();
    await state.load();
    await state.cancelActive();
    expect(cancelActiveTasks).toHaveBeenCalledOnce();
    expect(getTaskMaintenance).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("purges expired history and refreshes the overview", async () => {
    const { state, wrapper } = mountState();
    await state.load();
    await state.purge("expired");
    expect(purgeTasks).toHaveBeenCalledWith({ mode: "expired" });
    expect(getTaskMaintenance).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("refuses all-terminal purge while active tasks exist", async () => {
    const { state, wrapper } = mountState();
    await state.load();
    await state.purge("all_terminal");
    expect(purgeTasks).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("purges all terminal history when the queue is idle", async () => {
    getTaskMaintenance.mockResolvedValue({
      ...overview,
      stats: { ...overview.stats, active: 0 },
    } as never);
    const { state, wrapper } = mountState();
    await state.load();
    await state.purge("all_terminal");
    expect(purgeTasks).toHaveBeenCalledWith({ mode: "all_terminal" });
    wrapper.unmount();
  });
});
