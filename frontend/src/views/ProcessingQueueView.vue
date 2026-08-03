<script setup lang="ts">
import { computed, proxyRefs } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import TablePagination from "../components/TablePagination.vue";
import { useProcessingQueue } from "../composables/use-processing-queue";
import type { TaskKind, TaskResponse, TaskStatus } from "../services/api";
import { formatTimestamp } from "../utils/format";

const { t } = useI18n();
const queue = proxyRefs(useProcessingQueue({ t }));

const statuses: TaskStatus[] = ["queued", "running", "waiting", "succeeded", "failed", "cancelled"];
const kinds: TaskKind[] = ["source_sync", "text_batch", "file_batch", "url_batch", "delete_batch", "translation", "vector_rebuild"];
const stages = ["download", "storage", "docling", "embedding", "indexing", "translation", "sync", "delete", "finalize"];
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
  { accessorKey: "task_id", header: t("processingQueue.task") },
  { accessorKey: "kind", header: t("processingQueue.type") },
  { accessorKey: "group_path", header: t("processingQueue.group") },
  { accessorKey: "status", header: t("processingQueue.status") },
  { accessorKey: "stage", header: t("processingQueue.stage") },
  { id: "waiting", header: t("processingQueue.waiting") },
  { id: "progress", header: t("processingQueue.progress") },
  { id: "error", header: t("processingQueue.error") },
  { accessorKey: "updated_at", header: t("processingQueue.updatedAt") },
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
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col gap-3">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <h1 class="text-lg font-semibold text-color">{{ t("processingQueue.title") }}</h1>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <UButton v-if="queue.failedCount > 0" color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :loading="queue.bulkAction === 'retry'" :disabled="!!queue.bulkAction" :label="t('processingQueue.retryAll') + ' (' + queue.failedCount + ')'" @click="queue.confirmRetryAllFailed" />
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
    <div class="min-h-0 flex-1 overflow-auto">
      <AsyncStateBlock :loading="queue.loading && !queue.items.length" :error="queue.items.length ? null : queue.error" :loading-title="t('common.loading')">
        <template #error>
          <div class="grid justify-items-center gap-3 py-12 text-center">
            <UAlert color="error" variant="subtle" :title="t('processingQueue.loadFailed')" :description="queue.error || undefined" />
            <UButton color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :label="t('common.retry')" @click="queue.refresh" />
          </div>
        </template>
        <UTable class="min-w-[88rem]" :data="queue.items" :columns="columns" :loading="queue.loading">
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
              <UButton v-if="queue.isRetryableTask(row.original)" color="neutral" variant="ghost" size="sm" icon="i-lucide-rotate-ccw" :loading="queue.isActing(row.original)" :label="t('processingQueue.retry')" @click="queue.retryTask(row.original)" />
              <UButton v-if="['queued', 'running', 'waiting'].includes(row.original.status)" color="error" variant="ghost" size="sm" icon="i-lucide-ban" :loading="queue.isActing(row.original)" :aria-label="t('processingQueue.cancel')" :title="t('processingQueue.cancel')" @click="queue.cancelTask(row.original)" />
            </div>
          </template>
          <template #empty><div class="py-12 text-center text-sm text-muted">{{ t("processingQueue.noTasks") }}</div></template>
        </UTable>
      </AsyncStateBlock>
    </div>
    <TablePagination :pagination="queue.pagination" @update:page="queue.changePage($event)" @update:page-size="queue.changePageSize($event)" />
  </section>
</template>
