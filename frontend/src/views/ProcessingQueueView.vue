<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import AppServerList from "../components/AppServerList.vue";
import ProcessingQueueMaintenance from "../components/processing-queue/ProcessingQueueMaintenance.vue";
import ProcessingQueueTable from "../components/processing-queue/ProcessingQueueTable.vue";
import ProcessingQueueTabs from "../components/processing-queue/ProcessingQueueTabs.vue";
import { useProcessingQueue } from "../composables/use-processing-queue";
import { useTaskMaintenance } from "../composables/use-task-maintenance";
import type { TaskKind, TaskSortBy, TaskStatus } from "../services/api";
import { LIBRARY_DEPENDENCY_KEYS } from "../utils/library-status";

type QueueTab = "processing" | "completed" | "trash";

const { t } = useI18n();
const queue = proxyRefs(useProcessingQueue({ t }));
const maintenance = proxyRefs(useTaskMaintenance({
  t,
  onTasksChanged: () => { void queue.refresh(); },
}));

const AUTO_REFRESH_INTERVAL = 20_000;

// Processing is the live working set (the unfiltered active list, which
// already carries failed tasks for retry); Completed narrows it to the
// terminal `succeeded` status; Trash lists soft-deleted task history. The
// list API carries both a `status` and a `trashed` filter, so each tab is a
// single query and trashed rows never leak into the working views.
const activeTab = ref<QueueTab>("processing");

let refreshTimer: ReturnType<typeof setInterval> | null = null;

function startAutoRefresh() {
  stopAutoRefresh();
  refreshTimer = setInterval(() => {
    if (activeTab.value !== "processing") return;
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

watch(activeTab, (tab) => {
  if (tab === "trash") {
    stopAutoRefresh();
    queue.setListView({ trashed: true, status: null });
    return;
  }
  startAutoRefresh();
  queue.setListView({ trashed: false, status: tab === "completed" ? "succeeded" : null });
});

onMounted(() => {
  if (maintenance.isAdmin) void maintenance.load();
  startAutoRefresh();
});

onBeforeUnmount(stopAutoRefresh);

const statuses: TaskStatus[] = ["queued", "running", "waiting", "succeeded", "failed", "cancelled"];
const kinds: TaskKind[] = ["source_sync", "text_batch", "file_batch", "url_batch", "delete_batch", "translation", "vector_rebuild"];
const stages = ["download", "storage", "docling", "docling_poll", "embedding", "indexing", "translation", "sync", "delete", "finalize"];
const waitingReasons = ["dependency", "backoff", "external_job"];
const dependencies = LIBRARY_DEPENDENCY_KEYS;

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
const dependencyOptions = computed(() => [
  { label: t("processingQueue.allDependencies"), value: null },
  ...dependencies.map((value) => ({ label: t(`processingQueue.dependencies.${value}`), value })),
]);

function handleSort(value: { field: TaskSortBy; direction: "asc" | "desc" } | null) {
  if (!value) {
    queue.clearSort();
    return;
  }
  queue.changeSort(value.field, value.direction);
}
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col gap-3 overflow-hidden">
    <ProcessingQueueTabs v-model="activeTab" class="shrink-0" />

    <AppServerList
      data-testid="processing-queue-list"
      class="min-h-0 min-w-0 flex-1"
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
            <UButton v-if="activeTab !== 'trash' && queue.recoverableCount > 0" color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :loading="queue.bulkAction === 'recover'" :disabled="!!queue.bulkAction" :label="t('processingQueue.recoverAll') + ' (' + queue.recoverableCount + ')'" @click="queue.confirmRecoverAll" />
            <UButton v-if="activeTab !== 'trash' && queue.activeCount > 0" color="error" variant="outline" icon="i-lucide-ban" :loading="queue.bulkAction === 'cancel'" :disabled="!!queue.bulkAction" :label="t('processingQueue.cancelActive') + ' (' + queue.activeCount + ')'" @click="queue.confirmCancelActive" />
            <UButton color="neutral" variant="outline" icon="i-lucide-refresh-cw" :loading="queue.loading" :disabled="!!queue.bulkAction" :aria-label="t('processingQueue.refresh')" :title="t('processingQueue.refresh')" @click="queue.refresh" />
          </div>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <form class="flex min-w-64 max-w-full flex-1 gap-2" @submit.prevent="queue.submitSearch">
            <UInput v-model="queue.searchInput" class="min-w-0 flex-1" icon="i-lucide-search" :placeholder="t('processingQueue.searchPlaceholder')" />
            <UButton type="submit" color="neutral" variant="outline" icon="i-lucide-search" :aria-label="t('processingQueue.searchHint')" />
          </form>
          <USelect v-if="activeTab === 'processing'" :model-value="queue.statusFilter" :items="statusOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.statusFilter')" @update:model-value="queue.setStatusFilter($event as TaskStatus | null)" />
          <USelect :model-value="queue.kindFilter" :items="kindOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.kindFilter')" @update:model-value="queue.setKindFilter($event as TaskKind | null)" />
          <USelect :model-value="queue.stageFilter" :items="stageOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.stageFilter')" @update:model-value="queue.setStageFilter($event as string | null)" />
          <USelect :model-value="queue.waitingReasonFilter" :items="waitingReasonOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.waitingReasonFilter')" @update:model-value="queue.setWaitingReasonFilter($event as string | null)" />
          <USelect :model-value="queue.dependencyKeyFilter" :items="dependencyOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.dependencyFilter')" @update:model-value="queue.setDependencyKeyFilter($event as string | null)" />
        </div>

        <UAlert v-if="queue.error && queue.items.length" color="error" variant="subtle" :title="t('common.error')" :description="queue.error" />
      </template>

      <div
        v-if="queue.items.length"
        data-testid="processing-queue-table-scroll"
        class="h-full min-h-[220px] min-w-0 overflow-auto overscroll-contain"
      >
        <ProcessingQueueTable
          :items="queue.items"
          :loading="queue.loading"
          :is-admin="maintenance.isAdmin"
          :trash-view="activeTab === 'trash'"
          :is-acting="queue.isActing"
          :is-recoverable-task="queue.isRecoverableTask"
          :is-docling-recovery-task="queue.isDoclingRecoveryTask"
          @recover="queue.recoverTask"
          @recover-item="queue.recoverDoclingFromItem"
          @cancel="queue.cancelTask"
          @trash="queue.confirmTrashTask"
          @restore="queue.restoreTask"
          @delete="queue.confirmDeleteTask"
          @sort="handleSort"
        />
      </div>
      <div v-else-if="!queue.loading && !queue.error" class="py-12 text-sm text-muted">
        {{ t(activeTab === "completed" ? "processingQueue.tabs.noCompletedTasks" : activeTab === "trash" ? "processingQueue.tabs.noTrashedTasks" : "processingQueue.noTasks") }}
      </div>
    </AppServerList>

    <ProcessingQueueMaintenance
      v-if="maintenance.isAdmin"
      :error="maintenance.error"
      :stats="maintenance.stats"
      :active-count="maintenance.activeCount"
      :uncertain-submitting="maintenance.uncertainSubmitting"
      :quarantinable-submitting="maintenance.quarantinableSubmitting"
      :orphaned-external-jobs="maintenance.orphanedExternalJobs"
      :last-quarantine="maintenance.lastQuarantine"
      :action="maintenance.action"
      :saving="maintenance.saving"
      :settings="maintenance.settings"
      @confirm-cancel="maintenance.confirmCancelActive"
      @confirm-purge="maintenance.confirmPurge"
      @save-settings="maintenance.saveSettings"
      @quarantine="maintenance.quarantineStaleSubmitting"
    />
  </section>
</template>
