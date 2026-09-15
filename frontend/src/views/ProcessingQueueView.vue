<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import AppServerList from "../components/AppServerList.vue";
import ProcessingQueueTable from "../components/processing-queue/ProcessingQueueTable.vue";
import ProcessingQueueTabs from "../components/processing-queue/ProcessingQueueTabs.vue";
import { useProcessingQueue } from "../composables/use-processing-queue";
import { authSessionState } from "../services/auth/session";
import type { SortDirection, TaskKind, TaskListView, TaskSortBy, TaskStatus } from "../services/api";
import { LIBRARY_DEPENDENCY_KEYS } from "../utils/library-status";

type QueueTab = TaskListView;

const { t } = useI18n();
const queue = proxyRefs(useProcessingQueue({ t }));
const isAdmin = computed(() => authSessionState.user?.is_admin === true);

// Processing is the live working set (failed tasks stay retryable there);
// Completed and Trash are mutually exclusive typed views. Each tab passes a
// single `view` query and a user-selected status only narrows that view.
// The processing tab stays live through the task SSE stream (issue 405 Task
// E3); the 20s poll survives inside the queue composable as the automatic
// fallback when the stream errors or is unavailable.
const activeTab = ref<QueueTab>("processing");

function startLive() {
  if (activeTab.value !== "processing") return;
  queue.startLiveUpdates();
}

function stopLive() {
  queue.stopLiveUpdates();
}

watch(activeTab, (tab) => {
  if (tab !== "processing") {
    stopLive();
    queue.setListView({ view: tab === "completed" ? "completed" : "trash" });
    return;
  }
  queue.setListView({ view: "processing" });
  startLive();
});

onMounted(() => {
  startLive();
});

onBeforeUnmount(stopLive);

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

function handleSort(value: { field: TaskSortBy; direction: SortDirection } | null) {
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
            <UButton v-if="activeTab !== 'trash' && queue.recoverableCount > 0" color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :loading="queue.bulkAction === 'recover'" :disabled="!!queue.bulkAction || !!queue.clearAction" :label="t('processingQueue.recoverAll') + ' (' + queue.recoverableCount + ')'" @click="queue.confirmRecoverAll" />
            <UButton v-if="activeTab !== 'trash' && queue.activeCount > 0" color="error" variant="outline" icon="i-lucide-ban" :loading="queue.bulkAction === 'cancel'" :disabled="!!queue.bulkAction || !!queue.clearAction" :label="t('processingQueue.cancelActive') + ' (' + queue.activeCount + ')'" @click="queue.confirmCancelActive" />
            <UButton v-if="activeTab === 'completed'" data-testid="clear-completed-button" color="neutral" variant="outline" icon="i-lucide-trash-2" :loading="queue.clearAction === 'completed'" :disabled="!!queue.bulkAction || !!queue.clearAction" :label="t('processingQueue.clearCompleted')" @click="queue.confirmClearCompleted" />
            <UButton v-if="activeTab === 'trash'" data-testid="clear-trash-button" color="error" variant="outline" icon="i-lucide-trash-2" :loading="queue.clearAction === 'trash'" :disabled="!!queue.bulkAction || !!queue.clearAction" :label="t('processingQueue.clearTrash')" @click="queue.confirmClearTrash" />
            <UButton color="neutral" variant="outline" icon="i-lucide-refresh-cw" :loading="queue.loading" :disabled="!!queue.bulkAction || !!queue.clearAction" :aria-label="t('processingQueue.refresh')" :title="t('processingQueue.refresh')" @click="queue.refresh" />
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
          :is-admin="isAdmin"
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
  </section>
</template>
