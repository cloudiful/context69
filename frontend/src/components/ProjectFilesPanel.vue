<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { proxyRefs, ref } from "vue";
import { useI18n } from "vue-i18n";
import ContextMenu from "primevue/contextmenu";
import Dialog from "primevue/dialog";
import Message from "primevue/message";
import { useToast } from "primevue/usetoast";

import { appContextMenuPt } from "./app-context-menu";
import LibraryCreateFolderDialog from "./LibraryCreateFolderDialog.vue";
import LibraryCreateTextFileDialog from "./LibraryCreateTextFileDialog.vue";
import LibraryMoveDialog from "./LibraryMoveDialog.vue";
import LibraryPreviewPanel from "./LibraryPreviewPanel.vue";
import LibraryPreviewShell from "./LibraryPreviewShell.vue";
import LibraryResourceTable from "./LibraryResourceTable.vue";
import LibraryToolbar from "./LibraryToolbar.vue";
import ProjectSourceFolderDialog from "./ProjectSourceFolderDialog.vue";
import { useProjectLibraryActions } from "../composables/project-library/use-project-library-actions";
import { useProjectLibraryDetail } from "../composables/project-library/use-project-library-detail";
import { useLibraryPreview as useProjectLibraryPreview } from "../composables/library/use-library-preview";
import { useProjectLibraryTree } from "../composables/project-library/use-project-library-tree";
import { apiClient } from "../services/api";
import { createLibraryStatusHelpers } from "../utils/library-status";
import type { ExplorerEntry, FileExplorerEntry } from "../types/library";

const props = defineProps<{
  groupPath: string;
}>();

const { t } = useI18n();
const toast = useToast();
const { statusLabel } = createLibraryStatusHelpers();
const mapStatusLabel = (status: string) => statusLabel(status as "pending" | "running" | "succeeded" | "failed");
const sourceFolderDialogBusy = ref(false);
const sourceFolderDialogError = ref("");
const sourceFolderDialogOpen = ref(false);
const sourceFolderDialogTitle = ref("");
const sourceFolderDialogFolderId = ref<string | null>(null);
const sourceFolderDialogFolderName = ref("");
const sourceFolderDialogValue = ref("");

const tree = useProjectLibraryTree({
  groupPath: props.groupPath,
  statusLabel: mapStatusLabel,
  t,
});
const treeState = proxyRefs(tree);

const detail = useProjectLibraryDetail({
  groupPath: props.groupPath,
  loadTree: tree.loadTree,
  selectedFileId: tree.selectedFileId,
  t,
});
const detailState = proxyRefs(detail);

const preview = useProjectLibraryPreview({
  allowDockedPreview: false,
  detail: detail.detail,
  selectedFileId: tree.selectedFileId,
  selectedFolderSummary: tree.selectedFolderSummary,
  t,
});
const previewState = proxyRefs(preview);

const actions = useProjectLibraryActions({
  groupPath: props.groupPath,
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
    const items = [
      { label: t("library.openFolder"), icon: "pi pi-folder-open", command: () => { void treeState.selectFolder(entry.id); } },
      { label: t("library.newFolder"), icon: "pi pi-folder-plus", command: () => { actionsState.openCreateFolderDialog(entry.folder); } },
    ];
    if (entry.isSourceFolder) {
      items.push({ label: t("sources.sync"), icon: "pi pi-refresh", command: () => { void syncSourceFolder(entry.id); } });
    }
    if (!entry.isSourceRecordsFolder) {
      items.push({ label: t("common.move"), icon: "pi pi-arrows-alt", command: () => { actionsState.openMoveFolderDialog(entry.folder); } });
      items.push({ label: t("common.delete"), icon: "pi pi-trash", command: () => { void actionsState.deleteFolder(entry.folder); } });
    }
    return items;
  }
  const items = [
    { label: entry.isSourceConfigFile ? t("library.editSourceConfig") : t("library.preview"), icon: entry.isSourceConfigFile ? "pi pi-file-edit" : "pi pi-eye", command: () => { void openExplorerEntry(entry); } },
  ];
  if (!entry.isSourceConfigFile && !entry.isSourceRecordFile) {
    items.push({ label: t("common.move"), icon: "pi pi-arrows-alt", command: () => { actionsState.openMoveFileDialog(entry.file); } });
    items.push({ label: t("common.delete"), icon: "pi pi-trash", command: () => { void actionsState.deleteFile(entry.file); } });
  }
  return items;
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
      {
        label: t("library.newSourceFolder"),
        icon: "pi pi-database",
        command: () => { openCreateSourceFolderDialog(); },
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
  void openExplorerEntry(entry);
}

async function openExplorerEntry(entry: ExplorerEntry) {
  if (entry.kind === "folder") {
    treeState.toggleFolderExpansion(entry.id);
    await treeState.selectFolder(entry.id);
    return;
  }
  if (entry.isSourceConfigFile) {
    await openSourceConfigEditor(entry);
    return;
  }
  await actionsState.revealPreviewForFile(entry.id);
}

function defaultSourceConfigTemplate(folderName = "") {
  return JSON.stringify({
    source_key: folderName,
    display_name: "",
    description: "",
    example_queries: [],
    connection: "",
    sync_strategy: "cursor",
    connector_type: "postgres_sql",
    base_query: "",
    batch_size: 200,
  }, null, 2);
}

function openCreateSourceFolderDialog() {
  sourceFolderDialogError.value = "";
  sourceFolderDialogFolderId.value = null;
  sourceFolderDialogFolderName.value = "";
  sourceFolderDialogTitle.value = t("library.newSourceFolder");
  sourceFolderDialogValue.value = defaultSourceConfigTemplate();
  sourceFolderDialogOpen.value = true;
}

async function openSourceConfigEditor(entry: FileExplorerEntry) {
  sourceFolderDialogBusy.value = true;
  sourceFolderDialogError.value = "";
  try {
    const detail = await apiClient.getGroupLibraryFile(props.groupPath, entry.id);
    sourceFolderDialogFolderId.value = detail.folder_id ?? null;
    sourceFolderDialogFolderName.value = detail.folder_path.split("/").filter(Boolean).at(-1) ?? "";
    sourceFolderDialogTitle.value = t("library.editSourceConfig");
    sourceFolderDialogValue.value = detail.sections[0]?.preview_text || defaultSourceConfigTemplate(sourceFolderDialogFolderName.value);
    sourceFolderDialogOpen.value = true;
  } catch (error) {
    sourceFolderDialogError.value = error instanceof Error ? error.message : t("library.detailLoadFailed");
  } finally {
    sourceFolderDialogBusy.value = false;
  }
}

async function saveSourceFolderDialog(payload: { folderName: string; value: string }) {
  sourceFolderDialogBusy.value = true;
  sourceFolderDialogError.value = "";
  try {
    const sourceConfig = JSON.parse(payload.value);
    if (sourceFolderDialogFolderId.value) {
      await apiClient.updateGroupSourceFolderConfig(props.groupPath, sourceFolderDialogFolderId.value, sourceConfig);
    } else {
      await apiClient.createGroupSourceFolder(props.groupPath, {
        parent_folder_id: treeState.selectedFolder?.folder_id ?? null,
        folder_name: payload.folderName,
        source_config: sourceConfig,
      });
    }
    sourceFolderDialogOpen.value = false;
    await treeState.refreshLibrary(detailState.loadDetail);
    toast.add({
      severity: "success",
      summary: sourceFolderDialogFolderId.value ? t("common.save") : t("library.newSourceFolder"),
      detail: payload.folderName || sourceConfig.source_key,
      life: 2500,
    });
  } catch (error) {
    sourceFolderDialogError.value = error instanceof Error ? error.message : t("common.save");
  } finally {
    sourceFolderDialogBusy.value = false;
  }
}

async function syncSourceFolder(folderId: string | null) {
  if (!folderId) {
    return;
  }
  try {
    await apiClient.syncGroupSourceFolder(props.groupPath, folderId);
    await treeState.refreshLibrary(detailState.loadDetail);
    toast.add({
      severity: "success",
      summary: t("sources.sync"),
      detail: t("sources.syncing"),
      life: 2500,
    });
  } catch (error) {
    treeState.treeError = error instanceof Error ? error.message : t("sources.syncFailed");
  }
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
  <ContextMenu ref="resourceContextMenu" unstyled :pt="appContextMenuPt" :model="resourceMenuItems" @hide="treeState.resourceContextEntry = null" />
  <ContextMenu ref="surfaceContextMenu" unstyled :pt="appContextMenuPt" :model="surfaceMenuItems" />
  <input
    ref="uploadInput"
    class="sr-only"
    type="file"
    multiple
    accept=".pdf,.docx,.xlsx,.md,.txt,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,text/plain,text/markdown"
    @change="handleUploadInputChange"
  >
  <LibraryToolbar
    :breadcrumb-home="treeState.breadcrumbHome"
    :breadcrumb-items="treeState.breadcrumbItems"
    :count-label="treeState.filteredResourceCountLabel"
    :search-query="treeState.resourceSearchQuery"
    @update:search-query="treeState.resourceSearchQuery = $event"
  />
  <Message v-if="treeState.treeError" severity="error" :closable="false">{{ treeState.treeError }}</Message>

  <section class="library-workspace library-workspace-embedded">
    <LibraryResourceTable
      :create-folder-busy="actionsState.createFolderBusy"
      :create-source-folder-busy="sourceFolderDialogBusy"
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
      @create-source-folder="openCreateSourceFolderDialog()"
      @sync-source-folder="syncSourceFolder($event.id)"
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

  <ProjectSourceFolderDialog
    :open="sourceFolderDialogOpen"
    :busy="sourceFolderDialogBusy"
    :error="sourceFolderDialogError"
    :folder-name="sourceFolderDialogFolderName"
    :folder-name-readonly="!!sourceFolderDialogFolderId"
    :title="sourceFolderDialogTitle"
    :value="sourceFolderDialogValue"
    @cancel="sourceFolderDialogOpen = false"
    @confirm="saveSourceFolderDialog"
    @update:value="sourceFolderDialogValue = $event"
  />

  <Dialog
    v-model:visible="previewState.previewDialogVisible"
    modal
    :header="previewState.previewTitle"
    class="library-preview-dialog w-[min(96vw,58rem)]"
  >
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
