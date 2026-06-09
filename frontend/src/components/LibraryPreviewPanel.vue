<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Tag from "primevue/tag";

import type { LibraryFileDetailResponse } from "../services/api";
import type { FolderSummary } from "../types/library";
import { formatNumber, formatTimestamp } from "../utils/format";
import { createLibraryStatusHelpers } from "../utils/library-status";
import AppPanel from "./AppPanel.vue";
import AsyncStateBlock from "./AsyncStateBlock.vue";
import LibraryPreviewContent from "./LibraryPreviewContent.vue";
import AppStateMessage from "./AppStateMessage.vue";
import EmptyState from "./EmptyState.vue";

const props = defineProps<{
  activeSectionKey: string;
  detail: LibraryFileDetailResponse | null;
  detailError: string;
  detailLoading: boolean;
  selectedFileId: string | null;
  selectedFolderSummary: FolderSummary | null;
}>();

const emit = defineEmits<{
  "update:activeSectionKey": [value: string];
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
  <div class="library-preview-content">
    <AsyncStateBlock
      :loading="!!selectedFileId && detailLoading"
      :loading-title="t('common.loading')"
      :loading-message="t('library.loadingDetail')"
      :error="selectedFileId ? detailError : ''"
      :error-title="t('library.previewUnavailableTitle')"
      :empty="!selectedFileId"
    >
      <template #empty>
        <div v-if="selectedFolderSummary" class="space-y-4">
          <AppPanel class="library-preview-card" :title="selectedFolderSummary.name">
            <div class="library-preview-section">
              <div class="library-preview-row">
                <span class="library-preview-row-label">{{ t("library.pathLabel") }}</span>
                <span class="library-preview-row-value">{{ selectedFolderSummary.path }}</span>
              </div>
              <div class="library-preview-row">
                <span class="library-preview-row-label">{{ t("library.contentLabel") }}</span>
                <div class="text-right">
                  <p class="library-preview-row-value">
                    {{ selectedFolderSummary.childFolderCount }} folders · {{ selectedFolderSummary.fileCount }} files
                  </p>
                </div>
              </div>
              <div class="library-preview-row">
                <span class="library-preview-row-label">{{ t("library.statusLabel") }}</span>
                <div class="text-right">
                  <p class="library-preview-row-value">
                    {{
                      selectedFolderSummary.processingCount > 0
                        ? t("library.folderProcessing", { count: selectedFolderSummary.processingCount })
                        : t("library.idleStatus")
                    }}
                  </p>
                </div>
              </div>
            </div>
          </AppPanel>

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

      <div v-if="selectedFileId && detail" class="space-y-4">
        <AppPanel class="library-preview-card" :title="detail.filename">
          <div class="library-preview-section">
            <div class="library-preview-row">
              <span class="library-preview-row-label">{{ t("library.pathLabel") }}</span>
              <span class="library-preview-row-value">{{ detail.folder_path }}</span>
            </div>
            <div class="library-preview-grid">
              <div class="library-preview-block">
                <p class="form-label">{{ t("library.statusLabel") }}</p>
                <Tag :value="statusLabel(detail.ingest_status)" :severity="statusSeverity(detail.ingest_status)" />
              </div>
              <div class="library-preview-block">
                <p class="form-label">{{ t("library.sizeLabel") }}</p>
                <p class="library-preview-row-value">{{ formatNumber(detail.size_bytes) }} bytes</p>
              </div>
              <div class="library-preview-block">
                <p class="form-label">{{ t("library.updatedColumn") }}</p>
                <p class="library-preview-row-value">{{ formatTimestamp(detail.updated_at) }}</p>
              </div>
              <div class="library-preview-block">
                <p class="form-label">{{ t("library.ingestedAt") }}</p>
                <p class="library-preview-row-value">{{ formatTimestamp(detail.ingested_at) }}</p>
              </div>
            </div>
          </div>
        </AppPanel>

        <AppStateMessage
          v-if="detail.ingest_status === 'running' || detail.ingest_status === 'pending'"
          severity="warn"
          :title="t('library.processingTitle')"
        >
          {{ t("library.processingMessage") }}
        </AppStateMessage>
        <AppStateMessage
          v-else-if="detail.ingest_status === 'failed'"
          severity="error"
          :title="t('library.processingFailedTitle')"
        >
          {{ detail.error_message || t("library.failedMessage") }}
        </AppStateMessage>

        <AppPanel v-if="detail.jobs.length > 0" class="library-preview-card" :title="t('library.jobsTitle')">
          <div class="space-y-2">
            <div
              v-for="job in detail.jobs"
              :key="job.job_id"
              class="library-job-row"
            >
              <div class="min-w-0">
                <p class="truncate text-sm text-app-text">{{ job.job_id }}</p>
                <p class="text-xs text-app-text-dim">{{ formatTimestamp(job.updated_at) }}</p>
              </div>
              <Tag :value="statusLabel(job.status)" :severity="statusSeverity(job.status)" />
            </div>
          </div>
        </AppPanel>

        <AppPanel
          v-if="detail.sections.length > 0"
          class="library-preview-card"
          :title="activeSection?.title ?? t('library.previewSectionTitle')"
        >
          <template #actions>
            <div class="flex flex-wrap gap-2">
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
          </template>

          <div v-if="activeSection" class="library-preview-section">
            <div class="flex items-center justify-end">
              <p class="text-xs text-app-text-dim">
                {{ t("library.documentId", { id: activeSection.document_id }) }}
              </p>
            </div>
            <LibraryPreviewContent
              :content="activeSection.preview_text"
              :content-format="activeSection.content_format"
            />
          </div>
        </AppPanel>
      </div>
    </AsyncStateBlock>
  </div>
</template>
