<script setup lang="ts">
import type { LibraryFileDetailResponse } from "../services/api";
import type { FolderSummary } from "../types/library";
import LibraryPreviewPanel from "./LibraryPreviewPanel.vue";

const props = defineProps<{
  open: boolean;
  title: string;
  activeSectionKey: string;
  detail: LibraryFileDetailResponse | null;
  detailLoading: boolean;
  groupPath: string;
  selectedFileId: string | null;
  selectedFolderSummary: FolderSummary | null;
  releasable: boolean;
  releasing: boolean;
  retrying: boolean;
}>();

const emit = defineEmits<{
  "retry": [fileId: string];
  "release": [fileId: string];
  "update:activeSectionKey": [value: string];
  "update:open": [value: boolean];
}>();
</script>

<template>
  <UModal
    :open="props.open"
    :title="props.title"
    class="library-preview-dialog w-[min(96vw,72rem)] max-w-[min(96vw,72rem)]"
    @update:open="emit('update:open', $event)"
  >
    <template #body>
      <LibraryPreviewPanel
        :active-section-key="props.activeSectionKey"
        :detail="props.detail"
        :detail-loading="props.detailLoading"
        :group-path="props.groupPath"
        :selected-file-id="props.selectedFileId"
        :selected-folder-summary="props.selectedFolderSummary"
        :releasable="props.releasable"
        :releasing="props.releasing"
        :retrying="props.retrying"
        @retry="emit('retry', $event)"
        @release="emit('release', $event)"
        @update:active-section-key="emit('update:activeSectionKey', $event)"
      />
    </template>
  </UModal>
</template>
