<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "./AppTag.vue";

import type { LibraryFileDetailResponse } from "../services/api";
import type { FolderSummary } from "../types/library";
import { formatBytes, formatTimestamp } from "../utils/format";
import { createLibraryStatusHelpers } from "../utils/library-status";
import AsyncStateBlock from "./AsyncStateBlock.vue";
import LibraryPreviewContent from "./LibraryPreviewContent.vue";
import AppStateMessage from "./AppStateMessage.vue";
import EmptyState from "./EmptyState.vue";

const props = defineProps<{
  activeSectionKey: string;
  detail: LibraryFileDetailResponse | null;
  detailLoading: boolean;
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
          <section class="grid gap-4 rounded-xl border border-(--p-content-border-color)/65 bg-(--p-content-background)/35 px-4 py-4">
            <h2 class="truncate text-base font-semibold text-(--p-text-color)">{{ selectedFolderSummary.name }}</h2>

            <div class="grid gap-3">
              <div class="flex items-start justify-between gap-4 border-b border-(--p-content-border-color)/55 pb-3 last:border-b-0 last:pb-0">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.pathLabel") }}</span>
                <span class="min-w-0 break-all text-right text-sm text-(--p-text-color)">{{ selectedFolderSummary.path }}</span>
              </div>
              <div class="flex items-start justify-between gap-4 border-b border-(--p-content-border-color)/55 pb-3 last:border-b-0 last:pb-0">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.contentLabel") }}</span>
                <p class="text-right text-sm text-(--p-text-color)">
                  {{ selectedFolderSummary.childFolderCount }} folders · {{ selectedFolderSummary.fileCount }} files
                </p>
              </div>
              <div class="flex items-start justify-between gap-4 border-b border-(--p-content-border-color)/55 pb-3 last:border-b-0 last:pb-0">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.statusLabel") }}</span>
                <p class="text-right text-sm text-(--p-text-color)">
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
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.statusLabel") }}</dt>
            <dd><Tag :value="statusLabel(detail.ingest_status)" :severity="statusSeverity(detail.ingest_status)" /></dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.sizeLabel") }}</dt>
            <dd class="text-sm text-(--p-text-color)">{{ formatBytes(detail.size_bytes) }}</dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.updatedColumn") }}</dt>
            <dd class="text-sm text-(--p-text-color)">{{ formatTimestamp(detail.updated_at) }}</dd>
          </div>
          <div class="grid gap-1">
            <dt class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.ingestedAt") }}</dt>
            <dd class="text-sm text-(--p-text-color)">{{ detail.ingested_at ? formatTimestamp(detail.ingested_at) : "—" }}</dd>
          </div>
        </dl>

        <AppStateMessage
          v-if="detail.ingest_status === 'running' || detail.ingest_status === 'pending'"
          severity="warn"
          :title="t('library.processingTitle')"
        >
          {{ t("library.processingMessage") }}
        </AppStateMessage>
        <div v-else-if="detail.ingest_status === 'failed'" class="grid justify-items-start gap-3">
          <AppStateMessage
            severity="error"
            :title="t('library.processingFailedTitle')"
          >
            {{ detail.error_message || t("library.failedMessage") }}
          </AppStateMessage>
          <AppStateMessage
            v-if="!detail.source_available"
            severity="warn"
            :title="t('library.sourceMissingTitle')"
          >
            {{ t("library.sourceMissingMessage") }}
          </AppStateMessage>
          <Button
            v-else
            icon="pi pi-refresh"
            :label="retrying ? t('library.retrying') : t('common.retry')"
            :loading="retrying"
            :disabled="retrying"
            @click="emit('retry', detail.file_id)"
          />
        </div>

        <section v-if="detail.sections.length > 0" class="grid gap-3">
          <div v-if="detail.sections.length > 1" class="flex flex-wrap gap-2">
            <Button
              v-for="section in detail.sections"
              :key="section.section_key"
              :severity="activeSection?.section_key === section.section_key ? undefined : 'secondary'"
              :variant="activeSection?.section_key === section.section_key ? undefined : 'outlined'"
              size="small"
              @click="emit('update:activeSectionKey', section.section_key)"
            >
              {{ section.section_label }}
            </Button>
          </div>

          <LibraryPreviewContent
            v-if="activeSection"
            :content="activeSection.preview_text"
            :content-format="activeSection.content_format"
          />
        </section>

        <section v-if="detail.jobs.length > 0" class="grid gap-2">
          <h2 class="text-sm font-semibold text-(--p-text-color)">{{ t("library.jobsTitle") }}</h2>
          <DataTable :value="detail.jobs" data-key="job_id" size="small">
            <Column field="job_id" :header="t('library.jobsTitle')">
              <template #body="{ data }">
                <span class="block max-w-96 truncate font-mono text-xs text-(--p-text-color)" :title="data.job_id">
                  {{ data.job_id }}
                </span>
              </template>
            </Column>
            <Column field="updated_at" :header="t('library.updatedColumn')" class="w-40">
              <template #body="{ data }">
                <span class="whitespace-nowrap text-xs text-(--p-text-muted-color)">{{ formatTimestamp(data.updated_at) }}</span>
              </template>
            </Column>
            <Column field="status" :header="t('library.statusLabel')" class="w-24">
              <template #body="{ data }">
                <Tag :value="statusLabel(data.status)" :severity="statusSeverity(data.status)" />
              </template>
            </Column>
          </DataTable>
        </section>
      </div>
    </AsyncStateBlock>
  </div>
</template>
