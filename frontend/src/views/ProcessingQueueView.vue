<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import AppServerList from "../components/AppServerList.vue";
import { useProcessingQueue } from "../composables/use-processing-queue";
import { useTaskMaintenance } from "../composables/use-task-maintenance";
import { apiClient } from "../services/api";
import type { TaskItemResponse, TaskKind, TaskResponse, TaskSortBy, TaskStatus } from "../services/api";
import { formatTimestamp } from "../utils/format";

const { t } = useI18n();
const queue = proxyRefs(useProcessingQueue({ t }));
const maintenance = proxyRefs(useTaskMaintenance({
  t,
  onTasksChanged: () => { void queue.refresh(); },
}));

const AUTO_REFRESH_INTERVAL = 20_000;

const expandedRows = ref<Record<string, boolean>>({});
const expandedItems = ref<Record<string, TaskItemResponse[] | null>>({});
const expandedError = ref<Record<string, string | null>>({});
const expandingTaskId = ref<string | null>(null);

const sorting = ref<{ id: string; desc: boolean }[]>([]);

watch(sorting, (value) => {
  const next = value[0];
  if (!next) {
    queue.clearSort();
    return;
  }
  if (!["kind", "group_path", "status", "stage", "updated_at"].includes(next.id)) return;
  queue.changeSort(next.id as TaskSortBy, next.desc ? "desc" : "asc");
});

async function toggleExpand(row: { original: TaskResponse; id: string }) {
  const taskId = row.original.task_id;
  const next = !expandedRows.value[row.id];
  expandedRows.value = { ...expandedRows.value, [row.id]: next };
  if (!next || expandedItems.value[taskId] !== undefined) return;
  expandingTaskId.value = taskId;
  try {
    const response = await apiClient.getTaskItems(taskId, { limit: 100 });
    expandedItems.value = { ...expandedItems.value, [taskId]: response.items };
    expandedError.value = { ...expandedError.value, [taskId]: null };
  } catch (error) {
    expandedError.value = {
      ...expandedError.value,
      [taskId]: error instanceof Error ? error.message : String(error),
    };
  } finally {
    expandingTaskId.value = null;
  }
}

let refreshTimer: ReturnType<typeof setInterval> | null = null;

function startAutoRefresh() {
  stopAutoRefresh();
  refreshTimer = setInterval(() => {
    if (document.visibilityState === "visible") {
      void queue.refresh();
    }
  }, AUTO_REFRESH_INTERVAL);
}

function stopAutoRefresh() {
  if (refreshTimer) {
    clearInterval(refreshTimer);
    refreshTimer = null;
  }
}

onMounted(() => {
  if (maintenance.isAdmin) void maintenance.load();
  startAutoRefresh();
});

onBeforeUnmount(stopAutoRefresh);

watch(
  () => queue.items.map((task) => task.updated_at).join(","),
  () => {
    // Refresh expanded item lists for tasks that are still active.
    const visibleExpanded = Object.keys(expandedRows.value).filter(
      (rowId) => expandedRows.value[rowId],
    );
    const taskIds = visibleExpanded.map((rowId) => queue.items[Number(rowId)]?.task_id).filter(Boolean);
    for (const taskId of taskIds) {
      const task = queue.items.find((candidate) => candidate.task_id === taskId);
      if (!task || !["queued", "running", "waiting"].includes(task.status)) continue;
      void apiClient
        .getTaskItems(taskId, { limit: 100 })
        .then((response) => {
          expandedItems.value = { ...expandedItems.value, [taskId]: response.items };
        })
        .catch(() => undefined);
    }
  },
);

const draftCleanup = ref(true);
const draftRetentionDays = ref(30);
watch(() => maintenance.settings, (settings) => {
  if (!settings) return;
  draftCleanup.value = settings.cleanup_enabled;
  draftRetentionDays.value = settings.retention_days;
}, { immediate: true });
const settingsDirty = computed(() => {
  const settings = maintenance.settings;
  return !!settings && (settings.cleanup_enabled !== draftCleanup.value || settings.retention_days !== draftRetentionDays.value);
});
const retentionInvalid = computed(() => draftRetentionDays.value < 1 || draftRetentionDays.value > 3650);
function saveSettings() {
  if (!settingsDirty.value || retentionInvalid.value || maintenance.saving) return;
  void maintenance.saveSettings(draftCleanup.value, draftRetentionDays.value);
}

const statuses: TaskStatus[] = ["queued", "running", "waiting", "succeeded", "failed", "cancelled"];
const kinds: TaskKind[] = ["source_sync", "text_batch", "file_batch", "url_batch", "delete_batch", "translation", "vector_rebuild"];
const stages = ["download", "storage", "docling", "docling_poll", "embedding", "indexing", "translation", "sync", "delete", "finalize"];
const waitingReasons = ["dependency", "backoff", "external_job"];

const statusOptions = computed(() => [
  { label: t("processingQueue.allStatuses"), value: null },
  ...statuses.map((value) => ({ label: t(`processingQueue.statuses.${value}`), value })),
]);
const kindOptions = computed(() => [
  { label: t("processingQueue.allKinds"), value: null },
  ...kinds.map((value) => ({ label: t(`processingQueue.kinds.${value}`), value })),
]);
const stageOptions = computed(() => [
  { label: t("processingQueue.allStages"), value: null },
  ...stages.map((value) => ({ label: t(`processingQueue.stages.${value}`), value })),
]);
const waitingReasonOptions = computed(() => [
  { label: t("processingQueue.allWaitingReasons"), value: null },
  ...waitingReasons.map((value) => ({ label: t(`processingQueue.waitingReasons.${value}`), value })),
]);
const columns = computed<TableColumn<TaskResponse>[]>(() => [
  { id: "expand", enableHiding: false },
  { accessorKey: "task_id", header: t("processingQueue.task") },
  { accessorKey: "kind", header: t("processingQueue.type"), enableSorting: true },
  { accessorKey: "group_path", header: t("processingQueue.group"), enableSorting: true },
  { accessorKey: "status", header: t("processingQueue.status"), enableSorting: true },
  { accessorKey: "stage", header: t("processingQueue.stage"), enableSorting: true },
  { id: "waiting", header: t("processingQueue.waiting") },
  { id: "progress", header: t("processingQueue.progress") },
  { id: "error", header: t("processingQueue.error") },
  { accessorKey: "updated_at", header: t("processingQueue.updatedAt"), enableSorting: true },
  { id: "actions", header: t("processingQueue.actions") },
]);

function taskStatusLabel(status: TaskStatus) { return t(`processingQueue.statuses.${status}`); }
function taskKindLabel(kind: TaskKind) { return t(`processingQueue.kinds.${kind}`); }
function stageLabel(stage: string | null) { return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage"); }
function waitingLabel(reason: string | null, dependency: string | null) {
  if (!reason) return "--";
  const label = t(`processingQueue.waitingReasons.${reason}`);
  return dependency ? `${label}: ${dependency}` : label;
}
function statusSeverity(status: TaskStatus): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}

function itemSeverity(status: TaskItemResponse["status"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}

function externalJobLabel(job: TaskItemResponse["external_job"]): string {
  if (!job) return "--";
  return `${job.remote_task_id} · ${job.status}`;
}

function externalJobTitle(job: TaskItemResponse["external_job"]): string | undefined {
  if (!job) return undefined;
  const parts = [
    `${job.provider} ${job.remote_task_id}`,
    `status: ${job.remote_status ?? job.status}`,
    `submitted: ${job.submitted_at}`,
    job.last_polled_at ? `last polled: ${job.last_polled_at}` : null,
    job.deadline_at ? `deadline: ${job.deadline_at}` : null,
    job.error_message ? `error: ${job.error_message}` : null,
  ];
  return parts.filter(Boolean).join("\n");
}
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col gap-3">
    <AppServerList
      class="min-h-0 flex-1 overflow-auto"
      :loading="queue.loading && !queue.items.length"
      :error="queue.items.length ? null : queue.error"
      :pagination="queue.pagination"
      @retry="queue.refresh"
      @update:page="queue.changePage($event)"
      @update:page-size="queue.changePageSize($event)"
    >
      <template #toolbar>
        <div class="flex flex-wrap items-start justify-between gap-3">
          <h1 class="text-lg font-semibold text-color">{{ t("processingQueue.title") }}</h1>
          <div class="flex flex-wrap items-center justify-end gap-2">
            <UButton v-if="queue.recoverableCount > 0" color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :loading="queue.bulkAction === 'recover'" :disabled="!!queue.bulkAction" :label="t('processingQueue.recoverAll') + ' (' + queue.recoverableCount + ')'" @click="queue.confirmRecoverAll" />
            <UButton v-if="queue.activeCount > 0" color="error" variant="outline" icon="i-lucide-ban" :loading="queue.bulkAction === 'cancel'" :disabled="!!queue.bulkAction" :label="t('processingQueue.cancelActive') + ' (' + queue.activeCount + ')'" @click="queue.confirmCancelActive" />
            <UButton color="neutral" variant="outline" icon="i-lucide-refresh-cw" :loading="queue.loading" :disabled="!!queue.bulkAction" :aria-label="t('processingQueue.refresh')" :title="t('processingQueue.refresh')" @click="queue.refresh" />
          </div>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <form class="flex min-w-64 max-w-full flex-1 gap-2" @submit.prevent="queue.submitSearch">
            <UInput v-model="queue.searchInput" class="min-w-0 flex-1" icon="i-lucide-search" :placeholder="t('processingQueue.searchPlaceholder')" />
            <UButton type="submit" color="neutral" variant="outline" icon="i-lucide-search" :aria-label="t('processingQueue.searchHint')" />
          </form>
          <USelect :model-value="queue.statusFilter" :items="statusOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.statusFilter')" @update:model-value="queue.setStatusFilter($event as TaskStatus | null)" />
          <USelect :model-value="queue.kindFilter" :items="kindOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.kindFilter')" @update:model-value="queue.setKindFilter($event as TaskKind | null)" />
          <USelect :model-value="queue.stageFilter" :items="stageOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.stageFilter')" @update:model-value="queue.setStageFilter($event as string | null)" />
          <USelect :model-value="queue.waitingReasonFilter" :items="waitingReasonOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.waitingReasonFilter')" @update:model-value="queue.setWaitingReasonFilter($event as string | null)" />
        </div>

        <UAlert v-if="queue.error && queue.items.length" color="error" variant="subtle" :title="t('common.error')" :description="queue.error" />
      </template>

      <UTable
        v-model:sorting="sorting"
        class="min-w-[88rem]"
        v-model:expanded="expandedRows"
        :data="queue.items"
        :columns="columns"
        :loading="queue.loading"
        :sorting-options="{ manualSorting: true }"
      >
          <template #expand-cell="{ row }">
            <UButton
              variant="ghost"
              color="neutral"
              size="sm"
              icon="i-lucide-chevron-right"
              :class="{ 'rotate-90': row.getIsExpanded() }"
              :aria-label="row.getIsExpanded() ? t('processingQueue.collapse') : t('processingQueue.expand')"
              :aria-expanded="row.getIsExpanded()"
              :disabled="expandingTaskId === row.original.task_id"
              @click="toggleExpand(row)"
            />
          </template>
          <template #task_id-cell="{ row }"><span class="block max-w-64 truncate font-mono text-xs" :title="row.original.task_id">{{ row.original.task_id }}</span></template>
          <template #kind-cell="{ row }"><UBadge :label="taskKindLabel(row.original.kind)" color="neutral" variant="subtle" /></template>
          <template #group_path-cell="{ row }"><span class="block max-w-48 truncate" :title="row.original.group_path || undefined">{{ row.original.group_path || "--" }}</span></template>
          <template #status-cell="{ row }"><UBadge :label="taskStatusLabel(row.original.status)" :color="statusSeverity(row.original.status)" variant="subtle" /></template>
          <template #stage-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ stageLabel(row.original.stage) }}</span></template>
          <template #waiting-cell="{ row }"><span class="block max-w-48 truncate text-sm text-muted" :title="waitingLabel(row.original.waiting_reason, row.original.dependency_key)">{{ waitingLabel(row.original.waiting_reason, row.original.dependency_key) }}</span></template>
          <template #progress-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ row.original.progress.succeeded }}/{{ row.original.progress.total }}</span></template>
          <template #error-cell="{ row }"><span class="block max-w-80 truncate text-sm text-muted" :title="row.original.error_summary || undefined">{{ row.original.error_summary || "--" }}</span></template>
          <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.updated_at) }}</span></template>
          <template #actions-cell="{ row }">
            <div class="flex items-center gap-1">
              <UButton v-if="queue.isRecoverableTask(row.original)" color="neutral" variant="ghost" size="sm" icon="i-lucide-rotate-ccw" :loading="queue.isActing(row.original)" :label="t(row.original.status === 'cancelled' ? 'processingQueue.resubmit' : 'processingQueue.retry')" :title="row.original.status === 'cancelled' ? t('processingQueue.resubmitHint') : undefined" @click="queue.recoverTask(row.original)" />
              <UButton v-if="['queued', 'running', 'waiting'].includes(row.original.status)" color="error" variant="ghost" size="sm" icon="i-lucide-ban" :loading="queue.isActing(row.original)" :aria-label="t('processingQueue.cancel')" :title="t('processingQueue.cancel')" @click="queue.cancelTask(row.original)" />
            </div>
          </template>
          <template #empty><div class="py-12 text-center text-sm text-muted">{{ t("processingQueue.noTasks") }}</div></template>
          <template #expanded="{ row }">
            <div class="p-3">
              <template v-if="expandedItems[row.original.task_id] === undefined">
                <div v-if="expandedError[row.original.task_id]" class="text-sm text-(--ui-error)">
                  {{ t("processingQueue.itemsLoadFailed") }}
                </div>
                <div v-else class="text-sm text-muted">{{ t("common.loading") }}…</div>
              </template>
              <template v-else-if="(expandedItems[row.original.task_id]?.length ?? 0) === 0">
                <div class="text-sm text-muted">{{ t("processingQueue.noItems") }}</div>
              </template>
              <div v-else class="grid gap-1">
                <div
                  v-for="item in expandedItems[row.original.task_id]"
                  :key="item.item_id"
                  class="grid grid-cols-[minmax(0,1fr)_auto_auto_minmax(0,1fr)_auto_auto] items-center gap-3 rounded-md bg-surface-50 dark:bg-surface-900/40 px-3 py-1.5 text-sm"
                >
                  <span class="block truncate font-mono text-xs text-muted" :title="item.item_id">{{ item.item_id }}</span>
                  <UBadge :label="item.status" :color="itemSeverity(item.status)" variant="subtle" />
                  <span class="whitespace-nowrap text-xs text-muted">{{ stageLabel(item.stage) }}</span>
                  <span class="block truncate text-xs text-muted" :title="item.error_message || undefined">{{ item.error_message || "--" }}</span>
                  <span class="block max-w-56 truncate text-xs text-muted" :title="externalJobTitle(item.external_job)">{{ externalJobLabel(item.external_job) }}</span>
                  <span class="whitespace-nowrap text-xs text-muted">{{ t("processingQueue.attempts", { count: item.attempt_count }) }}</span>
                </div>
              </div>
            </div>
          </template>
        </UTable>
      </AppServerList>

    <section v-if="maintenance.isAdmin" class="flex flex-col gap-3 rounded-lg border border-default/70 p-3">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 class="text-sm font-semibold text-color">{{ t("taskMaintenance.title") }}</h2>
        <div class="flex flex-wrap items-center gap-4 text-sm text-muted">
          <span>{{ t("taskMaintenance.total") }}: {{ maintenance.stats?.total ?? "--" }}</span>
          <span>{{ t("taskMaintenance.active") }}: {{ maintenance.activeCount }}</span>
          <span>{{ t("taskMaintenance.expiredTerminal") }}: {{ maintenance.stats?.expired_terminal ?? "--" }}</span>
        </div>
      </div>
      <UAlert v-if="maintenance.error" color="error" variant="subtle" :title="t('common.error')" :description="maintenance.error" />
      <div class="flex flex-wrap items-end justify-between gap-3">
        <div class="flex flex-wrap items-end gap-4">
          <label class="flex items-center gap-2 text-sm text-color">
            <USwitch :model-value="draftCleanup" :disabled="!maintenance.settings || maintenance.saving" data-testid="maintenance-cleanup-toggle" @update:model-value="draftCleanup = $event as boolean" />
            {{ t("taskMaintenance.autoCleanup") }}
          </label>
          <div class="w-36">
            <AppNumberField :input-id="'maintenance-retention'" :label="t('taskMaintenance.retentionDays')" :model-value="draftRetentionDays" :min="1" :max="3650" :disabled="!maintenance.settings || maintenance.saving" :test-id="'maintenance-retention'" @update:model-value="draftRetentionDays = $event ?? 30" />
          </div>
          <UButton color="neutral" variant="outline" icon="i-lucide-save" :loading="maintenance.saving" :disabled="!settingsDirty || retentionInvalid" :label="t('common.save')" @click="saveSettings" />
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <UButton color="error" variant="outline" icon="i-lucide-ban" :loading="maintenance.action === 'cancel'" :disabled="maintenance.activeCount === 0 || !!maintenance.action" :label="t('taskMaintenance.cancelActiveAction') + ' (' + maintenance.activeCount + ')'" @click="maintenance.confirmCancelActive" />
          <UButton color="neutral" variant="outline" icon="i-lucide-trash-2" :loading="maintenance.action === 'purge'" :disabled="!!maintenance.action" :label="t('taskMaintenance.purgeExpiredAction')" @click="maintenance.confirmPurge('expired')" />
          <UButton color="error" variant="outline" icon="i-lucide-trash-2" :loading="maintenance.action === 'purge'" :disabled="maintenance.activeCount > 0 || !!maintenance.action" :label="t('taskMaintenance.purgeAllAction')" @click="maintenance.confirmPurge('all_terminal')" />
        </div>
      </div>
    </section>
  </section>
</template>
