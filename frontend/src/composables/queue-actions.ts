import type { ComputedRef, Ref } from "vue";
import type { useToast } from "@nuxt/ui/composables";

import { apiClient, type ClearTaskHistoryView, type TaskResponse, type TaskStatus } from "../services/api";
import { ACTIVE_STATUSES, isRecoverableTask, isTerminalTask, summarizeResults } from "./queue-helpers";
import type { useAppConfirm } from "./use-app-confirm";
import type { useErrorToast } from "./use-error-toast";

export interface UseQueueActionsOptions {
  // Queue state owned by `useProcessingQueue`; shared, never copied.
  items: Ref<TaskResponse[]>;
  actionTaskIds: Ref<string[]>;
  bulkAction: Ref<"recover" | "cancel" | null>;
  clearAction: Ref<ClearTaskHistoryView | null>;
  statusFilter: Ref<TaskStatus | null>;
  recoverableCount: ComputedRef<number>;
  activeCount: ComputedRef<number>;
  failedCount: ComputedRef<number>;
  cancelledCount: ComputedRef<number>;
  // Shared collaborators plus the caller's load/toast/confirm/i18n wiring.
  load: () => Promise<void>;
  toast: ReturnType<typeof useToast>;
  confirm: ReturnType<typeof useAppConfirm>;
  showErrorToast: ReturnType<typeof useErrorToast>;
  t: (key: string, params?: Record<string, unknown>) => string;
}

/** Single-item, bulk, and clear-history actions for the processing queue.
 * Owns no state: reentrancy and bulk/clear exclusion stay on the caller's refs. */
export function useQueueActions(options: UseQueueActionsOptions) {
  const {
    items, actionTaskIds, bulkAction, clearAction, statusFilter,
    recoverableCount, activeCount, failedCount, cancelledCount,
    load, toast, confirm, showErrorToast, t,
  } = options;

  function isActing(task: TaskResponse) {
    return actionTaskIds.value.includes(task.task_id);
  }

  function beginAction(task: TaskResponse) {
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
  }

  function endAction(task: TaskResponse) {
    actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
  }

  // Shared body of the immediate single-item mutations: guard, mark in flight,
  // mutate, refresh, toast; release the task in `finally`.
  async function runTaskAction(
    task: TaskResponse,
    eligible: boolean,
    mutate: () => Promise<unknown>,
    successKey: string,
    failureKey: string,
  ) {
    if (!eligible || isActing(task)) return;
    beginAction(task);
    try {
      await mutate();
      await load();
      toast.add({ color: "success", title: t(successKey), description: task.task_id, duration: 2500 });
    } catch (actionError) {
      showErrorToast(actionError, t(failureKey));
    } finally {
      endAction(task);
    }
  }

  // Failed task retries in place; a cancelled task is resumed in place too, so
  // one submission never grows a second visible task.
  async function recoverTask(task: TaskResponse) {
    if (!isRecoverableTask(task) || isActing(task)) return;
    beginAction(task);
    try {
      const resumed = task.status === "cancelled";
      // Resume reopens the task's own cancelled and failed items and keeps the
      // record; retry requeues the failed items of a failed task.
      if (resumed) await apiClient.rerunTask(task.task_id);
      else await apiClient.retryTask(task.task_id);
      // A resumed task leaves its terminal status, so a narrowing status filter
      // would hide the row the user just recovered (446).
      if (resumed && statusFilter.value !== null) statusFilter.value = null;
      await load();
      toast.add({
        color: "success",
        title: t(resumed ? "processingQueue.resumeAccepted" : "processingQueue.retryAccepted"),
        description: task.task_id,
        duration: 2500,
      });
    } catch (recoverError) {
      showErrorToast(recoverError, t(task.status === "cancelled"
        ? "processingQueue.resumeFailed"
        : "processingQueue.retryFailed"));
    } finally {
      endAction(task);
    }
  }

  function cancelTask(task: TaskResponse) {
    return runTaskAction(task, ACTIVE_STATUSES.includes(task.status), () => apiClient.cancelTask(task.task_id), "processingQueue.cancelAccepted", "processingQueue.cancelFailed");
  }

  // Trash a terminal task's history; the backend rejects active rows.
  function trashTask(task: TaskResponse) {
    return runTaskAction(task, isTerminalTask(task), () => apiClient.trashTask(task.task_id), "processingQueue.trashAccepted", "processingQueue.trashFailed");
  }

  function confirmTrashTask(task: TaskResponse) {
    if (!isTerminalTask(task) || isActing(task)) return;
    confirm.require({
      header: t("processingQueue.trash"),
      message: t("processingQueue.trashConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.trashAction"),
      accept: () => void trashTask(task),
    });
  }

  function restoreTask(task: TaskResponse) {
    return runTaskAction(task, true, () => apiClient.restoreTask(task.task_id), "processingQueue.restoreAccepted", "processingQueue.restoreFailed");
  }

  // Permanent removal from the recycle bin; files and results are untouched.
  function deletePermanently(task: TaskResponse) {
    return runTaskAction(task, true, () => apiClient.deleteTask(task.task_id), "processingQueue.deleteAccepted", "processingQueue.deleteFailed");
  }

  function confirmDeleteTask(task: TaskResponse) {
    if (isActing(task)) return;
    confirm.require({
      header: t("processingQueue.deletePermanently"),
      message: t("processingQueue.deletePermanentlyConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.deletePermanentlyAction"),
      accept: () => void deletePermanently(task),
    });
  }

  // Bulk recovery resumes a cancelled task in place and retries a failed one in
  // place. Reports whether a resume happened, because only a resume moves the
  // record out of a terminal status that a narrowing filter would keep hidden.
  async function submitRecovery(task: TaskResponse): Promise<boolean> {
    if (task.status === "cancelled") {
      await apiClient.rerunTask(task.task_id);
      return true;
    }
    await apiClient.retryTask(task.task_id);
    return false;
  }

  function showBulkSummary(summary: ReturnType<typeof summarizeResults>, description?: string) {
    toast.add({
      color: summary.failed === 0 ? "success" : "warning",
      title: t("processingQueue.bulkSummary", {
        succeeded: summary.succeeded,
        skipped: summary.skipped,
        failed: summary.failed,
      }),
      description,
      duration: 3500,
    });
  }

  async function recoverAll() {
    if (recoverableCount.value === 0 || bulkAction.value || clearAction.value) return;
    bulkAction.value = "recover";
    try {
      const tasks = items.value.filter(isRecoverableTask);
      const results = await Promise.allSettled(tasks.map((task) => submitRecovery(task)));
      const summary = summarizeResults(results, /no retryable/i);
      const resumed = results.some((result) => result.status === "fulfilled" && result.value);
      // A resumed task leaves its terminal status: drop a narrowing status
      // filter so it is visible instead of stranding the old filtered view (446).
      if (resumed && statusFilter.value !== null) {
        statusFilter.value = null;
      }
      await load();
      showBulkSummary(
        summary,
        resumed ? t("processingQueue.bulkResumed", { count: recoverableCount.value }) : undefined,
      );
    } catch (recoverError) {
      showErrorToast(recoverError, t("processingQueue.bulkRetryFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  async function cancelActive() {
    if (activeCount.value === 0 || bulkAction.value || clearAction.value) return;
    bulkAction.value = "cancel";
    try {
      const results = await Promise.allSettled(items.value.filter((task) => ACTIVE_STATUSES.includes(task.status)).map((task) => apiClient.cancelTask(task.task_id)));
      const summary = summarizeResults(results);
      await load();
      showBulkSummary(summary);
    } catch (cancelError) {
      showErrorToast(cancelError, t("processingQueue.bulkCancelFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  function confirmRecoverAll() {
    if (recoverableCount.value === 0 || bulkAction.value || clearAction.value) return;
    confirm.require({
      header: t("processingQueue.retryAll"),
      message: t("processingQueue.recoverAllConfirm", {
        failed: failedCount.value,
        cancelled: cancelledCount.value,
      }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.retryAllAction"),
      accept: () => void recoverAll(),
    });
  }

  function confirmCancelActive() {
    if (activeCount.value === 0 || bulkAction.value || clearAction.value) return;
    confirm.require({
      header: t("processingQueue.cancelActive"),
      message: t("processingQueue.cancelActiveConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.cancelActiveAction"),
      accept: () => void cancelActive(),
    });
  }

  // User-scoped history clearing: `completed` removes the current user's
  // untrashed succeeded tasks, `trash` their trashed terminal tasks; active,
  // other users', and file/document/vector data are never touched.
  async function clearHistory(view: ClearTaskHistoryView) {
    if (clearAction.value || bulkAction.value) return;
    clearAction.value = view;
    try {
      const response = await apiClient.clearTaskHistory({ view });
      await load();
      toast.add({
        color: "success",
        title: t(view === "completed" ? "processingQueue.clearCompletedAccepted" : "processingQueue.clearTrashAccepted"),
        description: String(response.deleted_count),
        duration: 3000,
      });
    } catch (clearError) {
      showErrorToast(clearError, t(view === "completed" ? "processingQueue.clearCompletedFailed" : "processingQueue.clearTrashFailed"));
    } finally {
      clearAction.value = null;
    }
  }

  function confirmClearCompleted() {
    if (clearAction.value || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.clearCompleted"),
      message: t("processingQueue.clearCompletedConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.clearCompletedAction"),
      accept: () => void clearHistory("completed"),
    });
  }

  function confirmClearTrash() {
    if (clearAction.value || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.clearTrash"),
      message: t("processingQueue.clearTrashConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.clearTrashAction"),
      accept: () => void clearHistory("trash"),
    });
  }

  return {
    isActing,
    recoverTask,
    cancelTask,
    confirmTrashTask,
    restoreTask,
    confirmDeleteTask,
    recoverAll,
    cancelActive,
    confirmRecoverAll,
    confirmCancelActive,
    clearHistory,
    confirmClearCompleted,
    confirmClearTrash,
  };
}
