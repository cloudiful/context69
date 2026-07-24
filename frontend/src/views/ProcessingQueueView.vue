<script setup lang="ts">
import { computed, proxyRefs } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import TablePagination from "../components/TablePagination.vue";
import { useProcessingQueue } from "../composables/use-processing-queue";
import type { LibraryIngestFailureStage, LibraryIngestStatus, LibraryProcessingJobResponse } from "../services/api";
import { formatTimestamp } from "../utils/format";

const { t } = useI18n();
const queue = proxyRefs(useProcessingQueue({ t }));

const statusOptions = computed(() => [
  { label: t("processingQueue.allStatuses"), value: null },
  ...(["pending", "running", "succeeded", "failed"] as const).map((value) => ({
    label: t(`library.status.${value}`),
    value,
  })),
]);

const stageValues: LibraryIngestFailureStage[] = [
  "download",
  "storage",
  "docling",
  "parsing",
  "embedding",
  "indexing",
  "translation",
  "other",
];

const stageOptions = computed(() => [
  { label: t("processingQueue.allStages"), value: null },
  ...stageValues.map((value) => ({ label: t(`processingQueue.stages.${value}`), value })),
]);

const columns = computed<TableColumn<LibraryProcessingJobResponse>[]>(() => [
  { accessorKey: "filename", header: t("processingQueue.file") },
  { id: "group", header: t("processingQueue.group") },
  { id: "type", header: t("processingQueue.type") },
  { id: "status", header: t("processingQueue.status") },
  { id: "stage", header: t("processingQueue.stage"), meta: { class: { th: "whitespace-nowrap", td: "whitespace-nowrap" } } },
  { id: "error", header: t("processingQueue.error") },
  { accessorKey: "updated_at", header: t("processingQueue.updatedAt") },
  { id: "actions", header: t("processingQueue.actions") },
]);

function statusSeverity(status: LibraryIngestStatus): "success" | "error" | "warning" | "neutral" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "running") return "warning";
  return "neutral";
}

function statusLabel(status: LibraryIngestStatus) {
  return t(`library.status.${status}`);
}

function stageLabel(stage: LibraryIngestFailureStage | null | undefined) {
  return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage");
}

function itemLabel(item: LibraryProcessingJobResponse) {
  return item.filename || item.source_url || item.job_id;
}
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col gap-3">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h1 class="text-lg font-semibold text-color">{{ t("processingQueue.title") }}</h1>
      </div>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <UButton
          v-if="queue.summary.can_manage && queue.summary.retryable_failed_count > 0"
          color="neutral"
          variant="outline"
          icon="i-lucide-rotate-ccw"
          :loading="queue.bulkAction === 'retry'"
          :disabled="!!queue.bulkAction"
          :label="t('processingQueue.retryAll') + ' (' + queue.summary.retryable_failed_count + ')'"
          @click="queue.confirmRetryAllFailed"
        />
        <UButton
          v-if="queue.summary.can_manage && queue.summary.cleanupable_stuck_count > 0"
          color="error"
          variant="outline"
          icon="i-lucide-trash-2"
          :loading="queue.bulkAction === 'cleanup'"
          :disabled="!!queue.bulkAction"
          :label="t('processingQueue.cleanupStuck') + ' (' + queue.summary.cleanupable_stuck_count + ')'"
          @click="queue.confirmCleanupStuck"
        />
        <UButton color="neutral" variant="outline" icon="i-lucide-refresh-cw" :loading="queue.loading" :disabled="!!queue.bulkAction" :aria-label="t('processingQueue.refresh')" :title="t('processingQueue.refresh')" @click="queue.refresh" />
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <form class="flex min-w-64 max-w-full flex-1 gap-2" @submit.prevent="queue.submitSearch">
        <UInput v-model="queue.searchInput" class="min-w-0 flex-1" icon="i-lucide-search" :placeholder="t('processingQueue.searchPlaceholder')" />
        <UButton type="submit" color="neutral" variant="outline" icon="i-lucide-search" :aria-label="t('processingQueue.searchHint')" />
      </form>
      <USelect :model-value="queue.statusFilter" :items="statusOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.statusFilter')" @update:model-value="queue.setStatusFilter($event as LibraryIngestStatus | null)" />
      <USelect :model-value="queue.failureStageFilter" :items="stageOptions" value-key="value" class="w-44" :aria-label="t('processingQueue.stageFilter')" @update:model-value="queue.setFailureStageFilter($event as LibraryIngestFailureStage | null)" />
    </div>

    <UAlert v-if="queue.error && queue.items.length" color="error" variant="subtle" :title="t('common.error')" :description="queue.error" />

    <div class="min-h-0 flex-1 overflow-auto">
      <AsyncStateBlock
        :loading="queue.loading && !queue.items.length"
        :error="queue.items.length ? null : queue.error"
        :loading-title="t('common.loading')"
      >
        <template #error>
          <div class="grid justify-items-center gap-3 py-12 text-center">
            <UAlert color="error" variant="subtle" :title="t('processingQueue.loadFailed')" :description="queue.error || undefined" />
            <UButton color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :label="t('common.retry')" @click="queue.refresh" />
          </div>
        </template>

        <UTable class="min-w-[68rem]" :data="queue.items" :columns="columns" :loading="queue.loading">
          <template #filename-cell="{ row }">
            <span class="block max-w-64 truncate font-medium" :title="itemLabel(row.original)">{{ itemLabel(row.original) }}</span>
          </template>
          <template #group-cell="{ row }"><span class="block max-w-48 truncate" :title="row.original.group_path">{{ row.original.group_path }}</span></template>
          <template #type-cell="{ row }"><UBadge :label="t(row.original.kind === 'url_import' ? 'processingQueue.urlImport' : 'processingQueue.ingest')" color="neutral" variant="subtle" /></template>
          <template #status-cell="{ row }"><UBadge :label="statusLabel(row.original.status)" :color="statusSeverity(row.original.status)" variant="subtle" /></template>
          <template #stage-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ stageLabel(row.original.failure_stage) }}</span></template>
          <template #error-cell="{ row }">
            <span class="block max-w-80 truncate text-sm text-muted" :title="row.original.error_message || undefined">{{ row.original.error_message || "--" }}</span>
          </template>
          <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.updated_at) }}</span></template>
          <template #actions-cell="{ row }">
            <UButton
              v-if="row.original.can_retry && row.original.status === 'failed'"
              color="neutral"
              variant="ghost"
              size="sm"
              icon="i-lucide-rotate-ccw"
              :loading="queue.isRetrying(row.original)"
              :label="t('processingQueue.retry')"
              @click="queue.retryJob(row.original)"
            />
          </template>
          <template #empty><div class="py-12 text-center text-sm text-muted">{{ t("processingQueue.noJobs") }}</div></template>
        </UTable>
      </AsyncStateBlock>
    </div>

    <TablePagination
      :pagination="queue.pagination"
      @update:page="queue.changePage($event)"
      @update:page-size="queue.changePageSize($event)"
    />
  </section>
</template>
