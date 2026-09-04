import { computed, ref } from "vue";

import { useToast } from "@nuxt/ui/composables";

import {
  apiClient,
  type QuarantineStaleSubmittingResponse,
  type TaskMaintenanceOverview,
  type TaskPurgeMode,
} from "../services/api";
import { authSessionState } from "../services/auth/session";
import { useAppConfirm } from "./use-app-confirm";
import { errorMessage, useErrorToast } from "./use-error-toast";

interface UseTaskMaintenanceOptions {
  t: (key: string) => string;
  onTasksChanged?: () => void;
}

export function useTaskMaintenance({ t, onTasksChanged }: UseTaskMaintenanceOptions) {
  const showErrorToast = useErrorToast();
  const toast = useToast();
  const confirm = useAppConfirm();
  const overview = ref<TaskMaintenanceOverview | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const saving = ref(false);
  const action = ref<"cancel" | "purge" | "quarantine" | null>(null);
  const lastQuarantine = ref<QuarantineStaleSubmittingResponse | null>(null);

  const isAdmin = computed(() => authSessionState.user?.is_admin === true);
  const settings = computed(() => overview.value?.settings ?? null);
  const stats = computed(() => overview.value?.stats ?? null);
  const activeCount = computed(() => stats.value?.active ?? 0);
  const uncertainSubmitting = computed(() => stats.value?.uncertain_submitting ?? null);
  const quarantinableSubmitting = computed(() => stats.value?.quarantinable_submitting ?? null);
  const orphanedExternalJobs = computed(() => stats.value?.orphaned_external_jobs ?? null);

  async function load() {
    if (!isAdmin.value) return;
    loading.value = true;
    error.value = null;
    try {
      overview.value = await apiClient.getTaskMaintenance();
    } catch (loadError) {
      error.value = errorMessage(loadError, t("taskMaintenance.loadFailed"));
    } finally {
      loading.value = false;
    }
  }

  async function saveSettings(cleanupEnabled: boolean, retentionDays: number) {
    if (saving.value) return;
    saving.value = true;
    try {
      overview.value = await apiClient.updateTaskMaintenance({
        cleanup_enabled: cleanupEnabled,
        retention_days: retentionDays,
      });
      toast.add({ color: "success", title: t("taskMaintenance.settingsSaved"), duration: 2500 });
    } catch (saveError) {
      showErrorToast(saveError, t("taskMaintenance.settingsSaveFailed"));
    } finally {
      saving.value = false;
    }
  }

  async function cancelActive() {
    if (action.value) return;
    action.value = "cancel";
    try {
      const response = await apiClient.cancelActiveTasks();
      await load();
      onTasksChanged?.();
      toast.add({
        color: "success",
        title: t("taskMaintenance.cancelAccepted"),
        description: String(response.cancelled_tasks),
        duration: 3000,
      });
    } catch (cancelError) {
      showErrorToast(cancelError, t("taskMaintenance.cancelFailed"));
      await load();
    } finally {
      action.value = null;
    }
  }

  function confirmCancelActive() {
    if (action.value || activeCount.value === 0) return;
    confirm.require({
      header: t("taskMaintenance.cancelActive"),
      message: t("taskMaintenance.cancelActiveConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("taskMaintenance.cancelActiveAction"),
      accept: () => void cancelActive(),
    });
  }

  async function purge(mode: TaskPurgeMode) {
    if (action.value) return;
    if (mode === "all_terminal" && activeCount.value > 0) return;
    action.value = "purge";
    try {
      const response = await apiClient.purgeTasks({ mode });
      await load();
      onTasksChanged?.();
      toast.add({
        color: "success",
        title: mode === "all_terminal" ? t("taskMaintenance.purgeAllCompleted") : t("taskMaintenance.purgeExpiredCompleted"),
        description: String(response.deleted_tasks),
        duration: 3000,
      });
    } catch (purgeError) {
      showErrorToast(purgeError, t("taskMaintenance.purgeFailed"));
      await load();
    } finally {
      action.value = null;
    }
  }

  function confirmPurge(mode: TaskPurgeMode) {
    if (action.value) return;
    if (mode === "all_terminal" && activeCount.value > 0) return;
    confirm.require({
      header: mode === "all_terminal" ? t("taskMaintenance.purgeAll") : t("taskMaintenance.purgeExpired"),
      message: mode === "all_terminal" ? t("taskMaintenance.purgeAllConfirm") : t("taskMaintenance.purgeExpiredConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("taskMaintenance.purgeAction"),
      accept: () => void purge(mode),
    });
  }

  // Controlled stale-`submitting` quarantine. Only an explicit admin call
  // with a non-empty reason isolates placeholder rows older than the grace
  // cutoff on terminal parents as `orphaned`; nothing runs automatically
  // and the transition never claims the remote job was cancelled. The
  // response carries quarantined rows plus skip counts so the UI can show
  // exactly what stayed `submitting` and why.
  async function quarantineStaleSubmitting(reason: string, graceMinutes?: number, limit?: number) {
    if (action.value) return;
    const trimmed = reason.trim();
    if (!trimmed) {
      showErrorToast(new Error("empty reason"), t("taskMaintenance.quarantineReasonRequired"));
      return;
    }
    action.value = "quarantine";
    try {
      const response = await apiClient.quarantineStaleSubmitting({
        reason: trimmed,
        grace_minutes: graceMinutes ?? null,
        limit: limit ?? null,
      });
      lastQuarantine.value = response;
      await load();
      onTasksChanged?.();
      toast.add({
        color: "success",
        title: t("taskMaintenance.quarantineCompleted"),
        description: String(response.quarantined_count),
        duration: 3000,
      });
    } catch (quarantineError) {
      showErrorToast(quarantineError, t("taskMaintenance.quarantineFailed"));
      await load();
    } finally {
      action.value = null;
    }
  }

  return {
    isAdmin,
    overview,
    settings,
    stats,
    activeCount,
    uncertainSubmitting,
    quarantinableSubmitting,
    orphanedExternalJobs,
    lastQuarantine,
    loading,
    error,
    saving,
    action,
    load,
    saveSettings,
    cancelActive,
    confirmCancelActive,
    purge,
    confirmPurge,
    quarantineStaleSubmitting,
  };
}
