<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { proxyRefs, ref } from "vue";
import { useI18n } from "vue-i18n";
import ContextMenu from "primevue/contextmenu";
import Dialog from "primevue/dialog";
import IconField from "primevue/iconfield";
import InputIcon from "primevue/inputicon";
import InputText from "primevue/inputtext";
import { useToast } from "primevue/usetoast";

import { appContextMenuPt } from "./app-context-menu";
import LibraryCreateFolderDialog from "./LibraryCreateFolderDialog.vue";
import LibraryCreateTextFileDialog from "./LibraryCreateTextFileDialog.vue";
import LibraryMoveDialog from "./LibraryMoveDialog.vue";
import LibraryPreviewPanel from "./LibraryPreviewPanel.vue";
import LibraryPreviewShell from "./LibraryPreviewShell.vue";
import LibraryResourceTable from "./LibraryResourceTable.vue";
import ProjectSourceFolderDialog from "./ProjectSourceFolderDialog.vue";
import { useProjectLibraryActions } from "../composables/project-library/use-project-library-actions";
import { useProjectLibraryDetail } from "../composables/project-library/use-project-library-detail";
import { useGroupBrowserEntries } from "../composables/project-library/use-group-browser-entries";
import { useLibraryPreview as useProjectLibraryPreview } from "../composables/library/use-library-preview";
import { useProjectLibraryTree } from "../composables/project-library/use-project-library-tree";
import { apiClient, type GroupResponse } from "../services/api";
import { createLibraryStatusHelpers } from "../utils/library-status";
import type { ExplorerEntry, FileExplorerEntry, GroupExplorerEntry } from "../types/library";
import { useErrorToast } from "../composables/use-error-toast";

const props = defineProps<{
  childGroups: GroupResponse[];
  groupPath: string;
}>();

const emit = defineEmits<{
  "create-child-group": [];
  "delete-child-group": [GroupResponse];
  "edit-child-group": [GroupResponse];
  "move-child-group": [GroupResponse];
  "open-child-group": [GroupResponse];
}>();

const { t } = useI18n();
const toast = useToast();
const showErrorToast = useErrorToast();
const { statusLabel } = createLibraryStatusHelpers();
const mapStatusLabel = (status: string) => statusLabel(status as "pending" | "running" | "succeeded" | "failed");
const sourceFolderDialogBusy = ref(false);
const sourceFolderDialogOpen = ref(false);
const sourceFolderDialogTitle = ref("");
const sourceFolderDialogFolderId = ref<string | null>(null);
const sourceFolderDialogFolderName = ref("");
const sourceFolderDialogValue = ref("");

const tree = useProjectLibraryTree({
  groupPath: () => props.groupPath,
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
  updateExpandedForFolder: tree.updateExpandedForFolder,
  previewDocked: preview.previewDocked,
  previewDialogVisible: preview.previewDialogVisible,
});
const actionsState = proxyRefs(actions);

const { filteredGroupEntries } = useGroupBrowserEntries({
  childGroups: () => props.childGroups,
  libraryEntryCount: () => treeState.filteredExplorerEntries.length,
  query: () => treeState.resourceSearchQuery,
  t,
});

const resourceContextMenu = ref();
const groupContextMenu = ref();
const groupContextEntry = ref<GroupExplorerEntry | null>(null);
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

const groupMenuItems = computed(() => {
  const group = groupContextEntry.value;
  if (!group) return [];
  return [
    { label: t("common.open"), icon: "pi pi-folder-open", command: () => { emit("open-child-group", group.group); } },
    { label: t("common.edit"), icon: "pi pi-pencil", command: () => { emit("edit-child-group", group.group); } },
    { label: t("common.move"), icon: "pi pi-arrows-alt", command: () => { emit("move-child-group", group.group); } },
    { label: t("common.delete"), icon: "pi pi-trash", command: () => { emit("delete-child-group", group.group); } },
  ];
});

const surfaceMenuItems = computed(() => [
  {
    label: t("common.create"),
    icon: "pi pi-plus",
    items: [
      {
        label: t("groups.createChild"),
        icon: "pi pi-sitemap",
        command: () => { emit("create-child-group"); },
      },
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
  sourceFolderDialogFolderId.value = null;
  sourceFolderDialogFolderName.value = "";
  sourceFolderDialogTitle.value = t("library.newSourceFolder");
  sourceFolderDialogValue.value = defaultSourceConfigTemplate();
  sourceFolderDialogOpen.value = true;
}

async function openSourceConfigEditor(entry: FileExplorerEntry) {
  sourceFolderDialogBusy.value = true;
  try {
    const detail = await apiClient.getGroupLibraryFile(props.groupPath, entry.id);
    sourceFolderDialogFolderId.value = detail.folder_id ?? null;
    sourceFolderDialogFolderName.value = detail.folder_path.split("/").filter(Boolean).at(-1) ?? "";
    sourceFolderDialogTitle.value = t("library.editSourceConfig");
    sourceFolderDialogValue.value = detail.sections[0]?.preview_text || defaultSourceConfigTemplate(sourceFolderDialogFolderName.value);
    sourceFolderDialogOpen.value = true;
  } catch (error) {
    showErrorToast(error, t("library.detailLoadFailed"));
  } finally {
    sourceFolderDialogBusy.value = false;
  }
}

async function saveSourceFolderDialog(payload: { folderName: string; value: string }) {
  sourceFolderDialogBusy.value = true;
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
    showErrorToast(error, t("common.save"));
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
    showErrorToast(error, t("sources.syncFailed"));
  }
}

function handleExplorerRowContextMenu(event: { originalEvent: Event; data: ExplorerEntry }) {
  treeState.resourceContextEntry = event.data;
  resourceContextMenu.value?.show(event.originalEvent);
}

function handleGroupRowContextMenu(event: { originalEvent: Event; data: GroupExplorerEntry }) {
  groupContextEntry.value = event.data;
  groupContextMenu.value?.show(event.originalEvent);
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

watch(() => props.groupPath, () => {
  tree.resetTree();
  void tree.loadTree();
}, { immediate: true });

onBeforeUnmount(() => {
  detail.dispose();
});
</script>

<template>
  <ContextMenu ref="resourceContextMenu" unstyled :pt="appContextMenuPt" :model="resourceMenuItems" @hide="treeState.resourceContextEntry = null" />
  <ContextMenu ref="groupContextMenu" unstyled :pt="appContextMenuPt" :model="groupMenuItems" @hide="groupContextEntry = null" />
  <ContextMenu ref="surfaceContextMenu" unstyled :pt="appContextMenuPt" :model="surfaceMenuItems" />
  <input
    ref="uploadInput"
    class="sr-only"
    type="file"
    multiple
    accept=".pdf,.docx,.xlsx,.md,.txt,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,text/plain,text/markdown"
    @change="handleUploadInputChange"
  >
  <Teleport to="#app-route-actions">
    <IconField class="relative w-56 [&.p-iconfield]:w-full">
      <InputIcon class="pi pi-search pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-app-text-dim" />
      <InputText
        v-model="treeState.resourceSearchQuery"
        class="w-full !pl-10"
        :placeholder="t('nav.search')"
      />
    </IconField>
  </Teleport>

  <section class="library-workspace library-workspace-embedded h-full">
    <LibraryResourceTable
      :create-folder-busy="actionsState.createFolderBusy"
      :create-source-folder-busy="sourceFolderDialogBusy"
      :entries="treeState.filteredExplorerEntries"
      :error="treeState.treeError"
      :group-entries="filteredGroupEntries"
      hide-actions
      hide-group-paths
      compact
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
      @group-contextmenu="handleGroupRowContextMenu"
      @surface-contextmenu="handleSurfaceContextMenu"
      @open-entry="openExplorerEntry"
      @move-entry="actionsState.moveExplorerEntry"
      @delete-entry="actionsState.deleteExplorerEntry"
      @open-group="emit('open-child-group', $event.group)"
      @edit-group="emit('edit-child-group', $event.group)"
      @move-group="emit('move-child-group', $event.group)"
      @delete-group="emit('delete-child-group', $event.group)"
      @toggle-folder="treeState.toggleFolderExpansion($event.id)"
      @refresh="treeState.refreshLibrary(detailState.loadDetail)"
      @retry="treeState.loadTree"
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
      :detail-loading="detailState.detailLoading"
      :selected-file-id="treeState.selectedFileId"
      :selected-folder-summary="treeState.selectedFolderSummary"
      @update:active-section-key="detailState.activeSectionKey = $event"
    />
  </Dialog>
</template>
