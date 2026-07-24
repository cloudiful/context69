<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { TableColumn } from "@nuxt/ui";

import { apiClient, type LibraryFileDetailResponse, type LibraryIngestJobResponse, type Pagination } from "../services/api";
import type { FolderSummary } from "../types/library";
import { formatBytes, formatTimestamp } from "../utils/format";
import { createLibraryStatusHelpers } from "../utils/library-status";
import AsyncStateBlock from "./AsyncStateBlock.vue";
import LibraryPreviewContent from "./LibraryPreviewContent.vue";
import EmptyState from "./EmptyState.vue";
import TablePagination from "./TablePagination.vue";
import { useErrorToast } from "../composables/use-error-toast";

const props = defineProps<{
  activeSectionKey: string;
  detail: LibraryFileDetailResponse | null;
  detailLoading: boolean;
  groupPath?: string;
  selectedFileId: string | null;
  selectedFolderSummary: FolderSummary | null;
  retrying?: boolean;
}>();

const emit = defineEmits<{
  "update:activeSectionKey": [value: string];
  retry: [fileId: string];
}>();

const { t } = useI18n();
const { statusLabel, statusSeverity } = createLibraryStatusHelpers();

const activeSection = computed(() => {
  if (!props.detail) {
    return null;
  }

  return props.detail.sections.find((section) => section.section_key === props.activeSectionKey)
    ?? props.detail.sections[0]
    ?? null;
});
type LibraryJob = LibraryFileDetailResponse["jobs"][number];
const jobColumns = computed<TableColumn<LibraryJob>[]>(() => [
  { accessorKey: "job_id", header: t("library.jobsTitle") },
  { accessorKey: "updated_at", header: t("library.updatedColumn") },
  { accessorKey: "status", header: t("library.statusLabel") },
]);
const showErrorToast = useErrorToast();
const jobRows = ref<LibraryIngestJobResponse[]>([]);
const jobPage = ref(1);
const jobPageSize = ref(50);
const jobPagination = ref<Pagination>({ page: 1, page_size: 50, total: 0, total_pages: 0 });
const jobsLoading = ref(false);
let jobsController: AbortController | null = null;
let jobsRequestId = 0;

async function loadJobs(page = jobPage.value) {
  if (!props.selectedFileId) {
    jobRows.value = [];
    jobPagination.value = { page: 1, page_size: jobPageSize.value, total: 0, total_pages: 0 };
    return;
  }
  jobsController?.abort();
  const requestId = ++jobsRequestId;
  jobsController = new AbortController();
  jobsLoading.value = true;
  try {
    const response = props.groupPath
      ? await apiClient.getGroupLibraryFileJobs(props.groupPath, props.selectedFileId, { page, pageSize: jobPageSize.value }, { signal: jobsController.signal })
      : await apiClient.getLibraryFileJobs(props.selectedFileId, { page, pageSize: jobPageSize.value }, { signal: jobsController.signal });
    if (requestId !== jobsRequestId) return;
    jobPage.value = response.pagination.page;
    jobPageSize.value = response.pagination.page_size;
    jobRows.value = response.items;
    jobPagination.value = response.pagination;
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") return;
    if (requestId === jobsRequestId) showErrorToast(error, t("library.detailLoadFailed"));
  } finally {
    if (requestId === jobsRequestId) jobsLoading.value = false;
  }
}

function changeJobPage(page: number) {
  void loadJobs(page);
}

function changeJobPageSize(value: number) {
  if (jobPageSize.value === value) return;
  jobPageSize.value = value;
  jobPage.value = 1;
  void loadJobs(1);
}

watch(() => props.selectedFileId, () => {
  jobPage.value = 1;
  void loadJobs(1);
}, { immediate: true });

onBeforeUnmount(() => jobsController?.abort());
</script>

<template>
  <div class="grid gap-4">
    <AsyncStateBlock
      :loading="!!selectedFileId && detailLoading"
      :loading-title="t('common.loading')"
      :loading-message="t('library.loadingDetail')"
      :empty="!selectedFileId"
    >
      <template #empty>
        <div v-if="selectedFolderSummary" class="space-y-4">
          <section class="grid gap-4 rounded-xl border border-surface bg-surface-0 dark:bg-surface-950 px-4 py-4">
            <h2 class="truncate text-base font-semibold text-color">{{ selectedFolderSummary.name }}</h2>

            <div class="grid gap-3">
              <div class="flex items-start justify-between gap-4 border-b border-surface pb-3 last:border-b-0 last:pb-0">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.pathLabel") }}</span>
                <span class="min-w-0 break-all text-right text-sm text-color">{{ selectedFolderSummary.path }}</span>
              </div>
              <div class="flex items-start justify-between gap-4 border-b border-surface pb-3 last:border-b-0 last:pb-0">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.contentLabel") }}</span>
                <p class="text-right text-sm text-color">
                  {{ selectedFolderSummary.childFolderCount }} folders · {{ selectedFolderSummary.fileCount }} files
                </p>
              </div>
              <div class="flex items-start justify-between gap-4 border-b border-surface pb-3 last:border-b-0 last:pb-0">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.statusLabel") }}</span>
                <p class="text-right text-sm text-color">
                  {{
                    selectedFolderSummary.processingCount > 0
                      ? t("library.folderProcessing", { count: selectedFolderSummary.processingCount })
                      : t("library.idleStatus")
                  }}
                </p>
              </div>
            </div>
          </section>

          <EmptyState
            v-if="selectedFolderSummary.childFolderCount + selectedFolderSummary.fileCount === 0"
            :title="t('library.emptyFolderTitle')"
            :message="t('library.emptyFolderMessage')"
            variant="soft"
          />
        </div>

        <EmptyState
          v-else
          :title="t('library.noSelectionTitle')"
          :message="t('library.noSelectionMessage')"
          variant="soft"
        />
      </template>

      <div v-if="selectedFileId && detail" class="space-y-3">
        <dl class="grid grid-cols-[repeat(auto-fit,minmax(9.5rem,1fr))] gap-x-4 gap-y-2">
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.pathLabel") }}</dt>
            <dd class="break-all text-sm text-color">{{ detail.folder_path }}</dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.statusLabel") }}</dt>
            <dd><UBadge :label="statusLabel(detail.ingest_status)" :color="statusSeverity(detail.ingest_status)" /></dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.sizeLabel") }}</dt>
            <dd class="text-sm text-color">{{ formatBytes(detail.size_bytes) }}</dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.updatedColumn") }}</dt>
            <dd class="text-sm text-color">{{ formatTimestamp(detail.updated_at) }}</dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.ingestedAt") }}</dt>
            <dd class="text-sm text-color">{{ detail.ingested_at ? formatTimestamp(detail.ingested_at) : "—" }}</dd>
          </div>
        </dl>

        <UAlert
          v-if="detail.ingest_status === 'running' || detail.ingest_status === 'pending'"
          color="warning"
          variant="subtle"
          :title="t('library.processingTitle')"
          :description="t('library.processingMessage')"
        />
        <div v-else-if="detail.ingest_status === 'failed'" class="grid justify-items-start gap-3">
          <UAlert
            color="error"
            variant="subtle"
            :title="t('library.processingFailedTitle')"
            :description="detail.error_message || t('library.failedMessage')"
          />
          <UAlert
            v-if="!detail.source_available"
            color="warning"
            variant="subtle"
            :title="t('library.sourceMissingTitle')"
            :description="t('library.sourceMissingMessage')"
          />
          <UButton
            v-else
            :disabled="retrying"
            :aria-busy="retrying"
            @click="emit('retry', detail.file_id)"
          >
            <UIcon name="i-lucide-loader-circle" v-if="retrying" class="h-4 w-4" />
            <UIcon v-else name="i-lucide-refresh-cw" />
            <span>{{ retrying ? t("library.retrying") : t("common.retry") }}</span>
          </UButton>
        </div>

        <section v-if="detail.sections.length > 0" class="grid gap-3">
          <div v-if="detail.sections.length > 1" class="flex flex-wrap gap-2">
            <UButton
              v-for="section in detail.sections"
              :key="section.section_key"
              :color="activeSection?.section_key === section.section_key ? 'primary' : 'neutral'"
              :variant="activeSection?.section_key === section.section_key ? 'solid' : 'outline'"
              size="sm"
              @click="emit('update:activeSectionKey', section.section_key)"
            >
              {{ section.section_label }}
            </UButton>
          </div>

          <LibraryPreviewContent
            v-if="activeSection"
            :content="activeSection.preview_text"
            :content-format="activeSection.content_format"
          />
        </section>

        <section v-if="jobPagination.total > 0 || jobsLoading" class="grid gap-2">
          <h2 class="text-sm font-semibold text-color">{{ t("library.jobsTitle") }}</h2>
          <UTable class="min-w-0 max-w-full" :data="jobRows" :columns="jobColumns" :loading="jobsLoading">
            <template #job_id-cell="{ row }"><span class="block max-w-96 truncate font-mono text-xs" :title="row.original.job_id">{{ row.original.job_id }}</span></template>
            <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-xs text-muted">{{ formatTimestamp(row.original.updated_at) }}</span></template>
            <template #status-cell="{ row }"><UBadge :label="statusLabel(row.original.status)" :color="statusSeverity(row.original.status)" variant="subtle" /></template>
          </UTable>
          <TablePagination
            :pagination="jobPagination"
            @update:page="changeJobPage"
            @update:page-size="changeJobPageSize"
          />
        </section>
      </div>
    </AsyncStateBlock>
  </div>
</template>
