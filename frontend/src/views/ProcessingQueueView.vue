<script setup lang="ts">
import { onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import AppServerList from "../components/AppServerList.vue";
import ProcessingQueueTable from "../components/processing-queue/ProcessingQueueTable.vue";
import ProcessingQueueTabs from "../components/processing-queue/ProcessingQueueTabs.vue";
import { useProcessingQueue } from "../composables/use-processing-queue";
import type { SortDirection, TaskListView, TaskSortBy } from "../services/api";

type QueueTab = TaskListView;

const { t } = useI18n();
const queue = proxyRefs(useProcessingQueue({ t }));

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
        </div>

        <UAlert v-if="queue.error && queue.items.length" color="error" variant="subtle" :title="t('common.error')" :description="queue.error" />
      </template>

      <div
        v-if="queue.items.length || (!queue.loading && !queue.error)"
        data-testid="processing-queue-table-scroll"
        class="h-full min-h-[220px] min-w-0 overflow-auto overscroll-contain"
      >
        <ProcessingQueueTable
          :items="queue.items"
          :loading="queue.loading"
          :view="activeTab"
          :sort="queue.sort"
          :status-filter="queue.statusFilter"
          :kind-filter="queue.kindFilter"
          :stage-filter="queue.stageFilter"
          :waiting-reason-filter="queue.waitingReasonFilter"
          :dependency-key-filter="queue.dependencyKeyFilter"
          :is-acting="queue.isActing"
          :is-recoverable-task="queue.isRecoverableTask"
          @recover="queue.recoverTask"
          @cancel="queue.cancelTask"
          @trash="queue.confirmTrashTask"
          @restore="queue.restoreTask"
          @delete="queue.confirmDeleteTask"
          @sort="handleSort"
          @update:status-filter="queue.setStatusFilter"
          @update:kind-filter="queue.setKindFilter"
          @update:stage-filter="queue.setStageFilter"
          @update:waiting-reason-filter="queue.setWaitingReasonFilter"
          @update:dependency-key-filter="queue.setDependencyKeyFilter"
        />
      </div>
    </AppServerList>
  </section>
</template>
