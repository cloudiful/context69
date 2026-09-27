import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";
import { useAppConfirm } from "./use-app-confirm";

import { apiClient, type TaskResponse } from "../services/api";
import { useErrorToast } from "./use-error-toast";

const POLL_INTERVAL_MS = 1500;
const ACTIVE_TASK_STATUSES = ["queued", "running", "waiting"];

export function useSettingsVectorRebuild() {
  const { t } = useI18n();
  const toast = useToast();
  const confirm = useAppConfirm();
  const showErrorToast = useErrorToast();

  const vectorRebuildStatus = ref<TaskResponse | null>(null);
  let vectorRebuildTimer: ReturnType<typeof setTimeout> | undefined;

  async function loadVectorRebuildTask() {
    const response = await apiClient.listTasks({
      page: 1,
      pageSize: 1,
      view: "processing",
      kind: "vector_rebuild",
      status: null,
      stage: null,
      waitingReason: null,
      dependencyKey: null,
    });
    vectorRebuildStatus.value = response.items[0] ?? null;
    scheduleVectorRebuildPoll();
  }

  function scheduleVectorRebuildPoll() {
    clearTimeout(vectorRebuildTimer);
    if (!vectorRebuildStatus.value || !ACTIVE_TASK_STATUSES.includes(vectorRebuildStatus.value.status)) return;
    vectorRebuildTimer = setTimeout(async () => {
      try {
        await loadVectorRebuildTask();
      } catch (error) {
        showErrorToast(error, t("settings.runtime.vectorRebuildStatusFailed"));
      }
    }, POLL_INTERVAL_MS);
  }

  function clearVectorRebuildPoll() {
    clearTimeout(vectorRebuildTimer);
  }

  function confirmVectorIndexRebuild() {
    confirm.require({
      header: t("settings.runtime.vectorRebuild"),
      message: t("settings.runtime.vectorRebuildConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("settings.runtime.vectorRebuild"),
      accept: () => void startVectorIndexRebuild(),
    });
  }

  async function startVectorIndexRebuild() {
    try {
      const task = await apiClient.submitVectorIndexRebuild();
      vectorRebuildStatus.value = await apiClient.getTask(task.task_id);
      toast.add({ color: "info", title: t("settings.runtime.vectorRebuildStarted"), duration: 2500 });
      scheduleVectorRebuildPoll();
    } catch (error) {
      showErrorToast(error, t("settings.runtime.vectorRebuildFailed"));
    }
  }

  return {
    clearVectorRebuildPoll,
    confirmVectorIndexRebuild,
    loadVectorRebuildTask,
    scheduleVectorRebuildPoll,
    startVectorIndexRebuild,
    vectorRebuildStatus,
  };
}
