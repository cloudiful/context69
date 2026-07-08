<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { proxyRefs, ref } from "vue";
import { useI18n } from "vue-i18n";
import ContextMenu from "primevue/contextmenu";
import Dialog from "primevue/dialog";
import Message from "primevue/message";
import Splitter from "primevue/splitter";
import SplitterPanel from "primevue/splitterpanel";

import AppTableToolbar from "./AppTableToolbar.vue";
import LibraryCreateFolderDialog from "./LibraryCreateFolderDialog.vue";
import LibraryCreateTextFileDialog from "./LibraryCreateTextFileDialog.vue";
import LibraryMoveDialog from "./LibraryMoveDialog.vue";
import LibraryPreviewPanel from "./LibraryPreviewPanel.vue";
import LibraryPreviewShell from "./LibraryPreviewShell.vue";
import LibraryResourceTable from "./LibraryResourceTable.vue";
import { useProjectLibraryActions } from "../composables/project-library/use-project-library-actions";
import { useProjectLibraryDetail } from "../composables/project-library/use-project-library-detail";
import { useLibraryPreview as useProjectLibraryPreview } from "../composables/library/use-library-preview";
import { useProjectLibraryTree } from "../composables/project-library/use-project-library-tree";
import { createLibraryStatusHelpers } from "../utils/library-status";
import type { ExplorerEntry } from "../types/library";

const props = defineProps<{
  groupKey: string;
  projectKey: string;
}>();

const { t } = useI18n();
const { statusLabel } = createLibraryStatusHelpers();
const mapStatusLabel = (status: string) => statusLabel(status as "pending" | "running" | "succeeded" | "failed");

const tree = useProjectLibraryTree({
  groupKey: props.groupKey,
  projectKey: props.projectKey,
  statusLabel: mapStatusLabel,
  t,
});
const treeState = proxyRefs(tree);

const detail = useProjectLibraryDetail({
  groupKey: props.groupKey,
  projectKey: props.projectKey,
  loadTree: tree.loadTree,
  selectedFileId: tree.selectedFileId,
  t,
});
const detailState = proxyRefs(detail);

const preview = useProjectLibraryPreview({
  detail: detail.detail,
  selectedFileId: tree.selectedFileId,
  selectedFolderSummary: tree.selectedFolderSummary,
  t,
});
const previewState = proxyRefs(preview);

const actions = useProjectLibraryActions({
  groupKey: props.groupKey,
  projectKey: props.projectKey,
  loadTree: tree.loadTree,
  moveOptions: tree.moveOptions,
  replaceSelection: tree.replaceSelection,
  schedulePolling: detail.schedulePolling,
  selectFile: tree.selectFile,
  selectedFolder: tree.selectedFolder,
  selectedFileId: tree.selectedFileId,
  t,
  treeError: tree.treeError,
  updateExpandedForFolder: tree.updateExpandedForFolder,
  previewDocked: preview.previewDocked,
  previewDialogVisible: preview.previewDialogVisible,
});
const actionsState = proxyRefs(actions);

const resourceContextMenu = ref();
const surfaceContextMenu = ref();
const uploadInput = ref<HTMLInputElement | null>(null);
const resourceMenuItems = computed(() => {
  const entry = treeState.resourceContextEntry;
  if (!entry) return [];
  if (entry.kind === "folder") {
    return [
      { label: t("library.openFolder"), icon: "pi pi-folder-open", command: () => { void treeState.selectFolder(entry.id); } },
      { label: t("library.newFolder"), icon: "pi pi-folder-plus", command: () => { actionsState.openCreateFolderDialog(entry.folder); } },
      { label: t("common.move"), icon: "pi pi-arrows-alt", command: () => { actionsState.openMoveFolderDialog(entry.folder); } },
      { label: t("common.delete"), icon: "pi pi-trash", command: () => { void actionsState.deleteFolder(entry.folder); } },
    ];
  }
  return [
    { label: t("library.preview"), icon: "pi pi-eye", command: () => { void actionsState.revealPreviewForFile(entry.id); } },
    { label: t("common.move"), icon: "pi pi-arrows-alt", command: () => { actionsState.openMoveFileDialog(entry.file); } },
    { label: t("common.delete"), icon: "pi pi-trash", command: () => { void actionsState.deleteFile(entry.file); } },
  ];
});

const surfaceMenuItems = computed(() => [
  {
    label: t("common.create"),
    icon: "pi pi-plus",
    items: [
      {
        label: t("library.newFolder"),
        icon: "pi pi-folder-plus",
        command: () => { actionsState.openCreateFolderDialog(); },
      },
      {
        label: t("library.newTextFile"),
        icon: "pi pi-file-edit",
        command: () => { actionsState.openCreateTextDialog(); },
      },
    ],
  },
  {
    label: t("common.upload"),
    icon: "pi pi-upload",
    command: () => { uploadInput.value?.click(); },
  },
  {
    label: t("sources.refresh"),
    icon: "pi pi-refresh",
    command: () => { void treeState.refreshLibrary(detailState.loadDetail); },
  },
]);

function handleExplorerRowClick(event: { data: ExplorerEntry }) {
  const entry = event.data;
  treeState.selectedExplorerEntry = entry;
  if (entry.kind === "folder") {
    treeState.toggleFolderExpansion(entry.id);
    void treeState.selectFolder(entry.id);
  }
}

function handleExplorerRowDoubleClick(event: { data: ExplorerEntry }) {
  const entry = event.data;
  if (entry.kind === "folder") {
    treeState.toggleFolderExpansion(entry.id);
    void treeState.selectFolder(entry.id);
    return;
  }
  void actionsState.revealPreviewForFile(entry.id);
}

function openExplorerEntry(entry: ExplorerEntry) {
  if (entry.kind === "folder") {
    treeState.toggleFolderExpansion(entry.id);
    void treeState.selectFolder(entry.id);
    return;
  }
  void actionsState.revealPreviewForFile(entry.id);
}

function handleExplorerRowContextMenu(event: { originalEvent: Event; data: ExplorerEntry }) {
  treeState.resourceContextEntry = event.data;
  resourceContextMenu.value?.show(event.originalEvent);
}

function handleSurfaceContextMenu(event: { originalEvent: MouseEvent }) {
  treeState.resourceContextEntry = null;
  surfaceContextMenu.value?.show(event.originalEvent);
}

function handleUploadInputChange(event: Event) {
  const input = event.target;
  if (!(input instanceof HTMLInputElement)) {
    return;
  }

  actionsState.handleFileSelection({ files: Array.from(input.files ?? []) });
  input.value = "";
}

watch(tree.selectedFileId, (fileId) => {
  if (fileId && !previewState.previewDocked) {
    previewState.previewDialogVisible = true;
  }
  void detailState.loadDetail(fileId);
}, { immediate: true });

watch(tree.selectedFolderId, (folderId) => {
  treeState.updateExpandedForFolder(folderId);
});

watch(tree.explorerEntries, (entries) => {
  treeState.syncSelectedExplorerEntry(entries);
}, { immediate: true });

void tree.loadTree();

onBeforeUnmount(() => {
  detail.dispose();
});
</script>

<template>
  <ContextMenu ref="resourceContextMenu" :model="resourceMenuItems" @hide="treeState.resourceContextEntry = null" />
  <ContextMenu ref="surfaceContextMenu" :model="surfaceMenuItems" />
  <input
    ref="uploadInput"
    class="sr-only"
    type="file"
    multiple
    accept=".pdf,.docx,.xlsx,.md,.txt,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,text/plain,text/markdown"
    @change="handleUploadInputChange"
  >
  <AppTableToolbar
    :search-query="treeState.resourceSearchQuery"
    :search-placeholder="t('library.filterResourcesPlaceholder')"
    @update:search-query="treeState.resourceSearchQuery = $event"
  />
  <Message v-if="treeState.treeError" severity="error" :closable="false">{{ treeState.treeError }}</Message>

  <section
    class="library-workspace library-workspace-embedded"
    :class="{ 'library-workspace-docked': previewState.showDockedPreview }"
  >
      <Splitter v-if="previewState.showDockedPreview" class="library-splitter">
        <SplitterPanel :size="62" :min-size="42">
          <LibraryResourceTable
            :create-folder-busy="actionsState.createFolderBusy"
            :entries="treeState.filteredExplorerEntries"
            :expanded-keys="treeState.expandedTreeKeys"
            :loading="treeState.treeLoading"
            :resource-search-query="treeState.resourceSearchQuery"
            :selected-folder-ready="!!treeState.selectedFolder"
            :selection="treeState.selectedExplorerEntry"
            :table-context-selection="treeState.resourceContextEntry"
            :upload-busy="actionsState.uploadBusy"
            @update:selection="treeState.selectedExplorerEntry = $event"
            @update:tableContextSelection="treeState.resourceContextEntry = $event"
            @row-click="handleExplorerRowClick"
            @row-dblclick="handleExplorerRowDoubleClick"
            @row-contextmenu="handleExplorerRowContextMenu"
            @surface-contextmenu="handleSurfaceContextMenu"
            @open-entry="openExplorerEntry"
            @move-entry="actionsState.moveExplorerEntry"
            @delete-entry="actionsState.deleteExplorerEntry"
            @toggle-folder="treeState.toggleFolderExpansion($event.id)"
            @refresh="treeState.refreshLibrary(detailState.loadDetail)"
            @create-folder="actionsState.openCreateFolderDialog()"
            @upload-select="actionsState.handleFileSelection"
          />
        </SplitterPanel>
        <SplitterPanel :size="38" :min-size="28">
          <LibraryPreviewShell :title="previewState.previewTitle" class="library-docked-preview">
            <LibraryPreviewPanel
              :active-section-key="detailState.activeSectionKey"
              :detail="detailState.detail"
              :detail-error="detailState.detailError"
              :detail-loading="detailState.detailLoading"
              :selected-file-id="treeState.selectedFileId"
              :selected-folder-summary="treeState.selectedFolderSummary"
              @update:active-section-key="detailState.activeSectionKey = $event"
            />
          </LibraryPreviewShell>
        </SplitterPanel>
      </Splitter>

      <LibraryResourceTable
        v-else
        :create-folder-busy="actionsState.createFolderBusy"
        :entries="treeState.filteredExplorerEntries"
        :expanded-keys="treeState.expandedTreeKeys"
        :loading="treeState.treeLoading"
        :resource-search-query="treeState.resourceSearchQuery"
        :selected-folder-ready="!!treeState.selectedFolder"
        :selection="treeState.selectedExplorerEntry"
        :table-context-selection="treeState.resourceContextEntry"
        :upload-busy="actionsState.uploadBusy"
        @update:selection="treeState.selectedExplorerEntry = $event"
        @update:tableContextSelection="treeState.resourceContextEntry = $event"
        @row-click="handleExplorerRowClick"
        @row-dblclick="handleExplorerRowDoubleClick"
        @row-contextmenu="handleExplorerRowContextMenu"
        @surface-contextmenu="handleSurfaceContextMenu"
        @open-entry="openExplorerEntry"
        @move-entry="actionsState.moveExplorerEntry"
        @delete-entry="actionsState.deleteExplorerEntry"
        @toggle-folder="treeState.toggleFolderExpansion($event.id)"
        @refresh="treeState.refreshLibrary(detailState.loadDetail)"
        @create-folder="actionsState.openCreateFolderDialog()"
        @upload-select="actionsState.handleFileSelection"
      />
  </section>

  <LibraryCreateFolderDialog
    :open="!!actionsState.createDialog"
    :busy="actionsState.createFolderBusy"
    :parent-name="actionsState.createDialog?.parentFolderName ?? t('library.rootFolder')"
    @cancel="actionsState.createDialog = null"
    @confirm="actionsState.confirmCreateFolder"
  />

  <LibraryCreateTextFileDialog
    :open="!!actionsState.createTextDialog"
    :busy="actionsState.createFolderBusy"
    :parent-name="actionsState.createTextDialog?.parentFolderName ?? t('library.rootFolder')"
    @cancel="actionsState.createTextDialog = null"
    @confirm="actionsState.confirmCreateTextFile"
  />

  <LibraryMoveDialog
    :open="!!actionsState.moveDialog"
    :busy="actionsState.actionBusy"
    :title="actionsState.moveDialog?.kind === 'folder' ? t('library.moveFolderTitle', { name: actionsState.moveDialog?.name ?? '' }) : t('library.moveFileTitle', { name: actionsState.moveDialog?.name ?? '' })"
    :description="actionsState.moveDialog?.kind === 'folder' ? t('library.moveFolderDescription') : t('library.moveFileDescription')"
    :options="actionsState.filteredMoveOptions"
    :current-folder-id="actionsState.moveDialog?.currentFolderId ?? null"
    @cancel="actionsState.moveDialog = null"
    @confirm="actionsState.confirmMove"
  />

  <Dialog v-model:visible="previewState.previewDialogVisible" modal :header="previewState.previewTitle" class="library-preview-dialog">
    <LibraryPreviewPanel
      :active-section-key="detailState.activeSectionKey"
      :detail="detailState.detail"
      :detail-error="detailState.detailError"
      :detail-loading="detailState.detailLoading"
      :selected-file-id="treeState.selectedFileId"
      :selected-folder-summary="treeState.selectedFolderSummary"
      @update:active-section-key="detailState.activeSectionKey = $event"
    />
  </Dialog>
</template>
