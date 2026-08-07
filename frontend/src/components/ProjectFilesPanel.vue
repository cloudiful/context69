<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { proxyRefs, ref } from "vue";
import type { ContextMenuItem } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

import LibraryCreateFolderDialog from "./LibraryCreateFolderDialog.vue";
import LibraryCreateTextFileDialog from "./LibraryCreateTextFileDialog.vue";
import LibraryMoveDialog from "./LibraryMoveDialog.vue";
import LibraryPreviewPanel from "./LibraryPreviewPanel.vue";
import LibraryResourceTable from "./LibraryResourceTable.vue";
import LibraryToolbar from "./LibraryToolbar.vue";
import ProjectSourceFolderDialog from "./ProjectSourceFolderDialog.vue";
import { groupContextItems, resourceContextItems, surfaceContextItems } from "./project-files-context-menu";
import { useProjectLibraryActions } from "../composables/project-library/use-project-library-actions";
import { useProjectLibraryDetail } from "../composables/project-library/use-project-library-detail";
import { useProjectLibraryPage } from "../composables/project-library/use-project-library-page";
import { useGroupBrowserEntries } from "../composables/project-library/use-group-browser-entries";
import { useLibraryPreview as useProjectLibraryPreview } from "../composables/library/use-library-preview";
import { useProjectLibraryTree } from "../composables/project-library/use-project-library-tree";
import { useProjectSourceFolder } from "../composables/project-library/use-project-source-folder";
import type { GroupPageResponse, GroupResponse } from "../services/api";
import { createLibraryStatusHelpers } from "../utils/library-status";
import type { ExplorerEntry, GroupExplorerEntry } from "../types/library";

const props = defineProps<{
  childGroups: GroupResponse[];
  childGroupPage: GroupPageResponse;
  childGroupSearch: string;
  groupPath: string;
}>();

const emit = defineEmits<{
  "create-child-group": [];
  "delete-child-group": [GroupResponse];
  "edit-child-group": [GroupResponse];
  "move-child-group": [GroupResponse];
  "open-child-group": [GroupResponse];
  "child-group-page": [number];
  "child-group-page-size": [number];
  "update:child-group-search": [string];
}>();

type FileUploadController = {
  inputRef?: HTMLInputElement;
};

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { statusLabel } = createLibraryStatusHelpers();
const mapStatusLabel = (status: string) => statusLabel(status as "pending" | "running" | "succeeded" | "failed" | "cancelled");
const tree = useProjectLibraryTree({
  groupPath: () => props.groupPath,
  statusLabel: mapStatusLabel,
  t,
});
const treeState = proxyRefs(tree);
const page = useProjectLibraryPage({
  groupPath: () => props.groupPath,
  folder: tree.selectedFolder,
  t,
});
const pageState = proxyRefs(page);
async function refreshLibraryData() {
  await tree.loadTree();
  await page.loadPage();
}
const detail = useProjectLibraryDetail({
  groupPath: () => props.groupPath,
  t,
});
const detailState = proxyRefs(detail);
const sourceFolderState = proxyRefs(useProjectSourceFolder({
  groupPath: () => props.groupPath,
  selectedFolder: tree.selectedFolder,
  refreshLibrary: () => treeState.refreshLibrary(detailState.loadDetail),
  t,
}));
const preview = useProjectLibraryPreview({
  allowDockedPreview: false,
  detail: detail.detail,
  selectedFileId: tree.selectedFileId,
  selectedFolderSummary: tree.selectedFolderSummary,
  t,
});
const previewState = proxyRefs(preview);
const actions = useProjectLibraryActions({
  groupPath: () => props.groupPath,
  loadTree: refreshLibraryData,
  moveOptions: tree.moveOptions,
  replaceSelection: tree.replaceSelection,
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
  libraryEntryCount: () => pageState.entries.length,
  query: () => pageState.query,
  t,
});
const visibleGroupEntries = computed(() => treeState.selectedFolderId || pageState.page !== 1 || pageState.statusFilter
  ? []
  : filteredGroupEntries.value);
const visibleChildGroupPage = computed(() => treeState.selectedFolderId || pageState.page !== 1 || pageState.statusFilter
  ? undefined
  : props.childGroupPage);

const groupContextEntry = ref<GroupExplorerEntry | null>(null);
const fileUpload = ref<FileUploadController | null>(null);
const uploadFiles = ref<File[] | null>(null);
const resourceMenuItems = computed(() => resourceContextItems({
  entry: treeState.resourceContextEntry, t,
  unavailableFileIds: actionsState.unavailableFileIds, retryingFileIds: actionsState.retryingFileIds,
  open: (entry) => { void openExplorerEntry(entry); },
  selectFolder: (id) => { void treeState.selectFolder(id); },
  createFolder: (entry) => entry.kind === "folder" && actionsState.openCreateFolderDialog(entry.folder),
  syncFolder: (id) => { void sourceFolderState.sync(id); },
  move: (entry) => entry.kind === "folder" ? actionsState.openMoveFolderDialog(entry.folder) : entry.kind === "file" && actionsState.openMoveFileDialog(entry.file),
  remove: (entry) => entry.kind === "folder" ? void actionsState.deleteFolder(entry.folder) : entry.kind === "file" && void actionsState.deleteFile(entry.file),
  retry: (id) => { void actionsState.retryFile(id); },
}));
const groupMenuItems = computed(() => groupContextItems(groupContextEntry.value, t, (action, entry) => {
  if (action === "open") emit("open-child-group", entry.group);
  else if (action === "edit") emit("edit-child-group", entry.group);
  else if (action === "move") emit("move-child-group", entry.group);
  else emit("delete-child-group", entry.group);
}));
const createMenuItems = computed(() => [
  { label: t("library.newFolder"), icon: "i-lucide-folder-plus", onSelect: () => actionsState.openCreateFolderDialog() },
  { label: t("library.newTextFile"), icon: "i-lucide-file-plus", onSelect: () => actionsState.openCreateTextDialog() },
  { label: t("library.newSourceFolder"), icon: "i-lucide-database-plus", onSelect: () => sourceFolderState.openCreate() },
]);
const surfaceMenuItems = computed(() => surfaceContextItems(t, {
  createGroup: () => emit("create-child-group"), createFolder: () => actionsState.openCreateFolderDialog(),
  createText: () => actionsState.openCreateTextDialog(), createSource: () => sourceFolderState.openCreate(),
  upload: () => fileUpload.value?.inputRef?.click(), refresh: () => { void treeState.refreshLibrary(detailState.loadDetail); },
}));
const activeContextMenuItems = computed<ContextMenuItem[][]>(() => [
  (treeState.resourceContextEntry ? resourceMenuItems.value : groupContextEntry.value ? groupMenuItems.value : surfaceMenuItems.value) as ContextMenuItem[],
]);

function handleExplorerRowClick(event: { data: ExplorerEntry }) {
  const entry = event.data;
  treeState.selectedExplorerEntry = entry;
  if (entry.kind === "folder") {
    treeState.toggleFolderExpansion(entry.id);
    void treeState.selectFolder(entry.id);
    return;
  }
  void openExplorerEntry(entry);
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
    await sourceFolderState.openEditor(entry);
    return;
  }
  await actionsState.revealPreviewForFile(entry.id);
}

function handleExplorerRowContextMenu(event: { originalEvent: Event; data: ExplorerEntry }) {
  treeState.resourceContextEntry = event.data;
  groupContextEntry.value = null;
}

function retryExplorerEntry(entry: ExplorerEntry) {
  if (entry.kind === "file") {
    void actionsState.retryFile(entry.id);
  }
}

function handleGroupRowContextMenu(event: { originalEvent: Event; data: GroupExplorerEntry }) {
  groupContextEntry.value = event.data;
  treeState.resourceContextEntry = null;
}

function handleSurfaceContextMenu(event: { originalEvent: MouseEvent }) {
  treeState.resourceContextEntry = null;
  groupContextEntry.value = null;
}

watch(uploadFiles, (files) => {
  if (!files?.length) return;
  actionsState.handleFileSelection({ files });
  uploadFiles.value = null;
});

watch(tree.selectedFileId, (fileId) => {
  if (fileId && !previewState.previewDocked) {
    previewState.previewDialogVisible = true;
  }
  void detailState.loadDetail(fileId);
}, { immediate: true });

watch(tree.selectedFolderId, (folderId) => {
  treeState.updateExpandedForFolder(folderId);
  page.reset();
  void page.loadPage();
});

watch(detail.detail, (nextDetail) => {
  const fileId = tree.selectedFileId.value;
  if (!fileId || !nextDetail || nextDetail.file_id !== fileId) return;
  if (nextDetail.folder_id !== tree.selectedFolderId.value) {
    void tree.replaceSelection(nextDetail.folder_id ?? null, fileId);
  }
});

watch(page.entries, (entries) => {
  treeState.syncSelectedExplorerEntry(entries);
}, { immediate: true });

watch(() => props.groupPath, async () => {
  tree.resetTree();
  page.reset();
  await tree.loadTree();
  await page.loadPage();
}, { immediate: true });

watch(() => route.query.file, async (fileId) => {
  if (typeof fileId !== "string" || !fileId) return;
  if (treeState.selectedFileId === fileId) return;
  try {
    if (!treeState.tree) await treeState.loadTree();
    await actionsState.revealPreviewForFile(fileId);
  } catch {
    // Load failures are surfaced through the tree error state and toasts.
  }
}, { immediate: true });

watch(tree.selectedFileId, (fileId) => {
  const current = typeof route.query.file === "string" ? route.query.file : null;
  if (fileId === current) return;
  if (fileId) {
    void router.replace({ query: { ...route.query, file: fileId } });
  } else if (current && treeState.tree) {
    // Clear the deep link after the tree has settled so a pending deep-link
    // navigation is not torn down by the reset triggered by a group change.
    const { file: _file, ...rest } = route.query;
    void router.replace({ query: rest });
  }
});

let searchTimer: ReturnType<typeof setTimeout> | undefined;
watch(page.query, () => {
  if (!treeState.selectedFolderId && pageState.page === 1 && !pageState.statusFilter) {
    emit("update:child-group-search", pageState.query);
  }
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    pageState.page = 1;
    void page.loadPage();
  }, 250);
});

onBeforeUnmount(() => {
  clearTimeout(searchTimer);
  detail.dispose();
});
</script>

<template>
  <div class="project-files-panel h-full min-h-0">
    <UFileUpload
      ref="fileUpload"
      v-model="uploadFiles"
      class="sr-only"
      multiple
      :preview="false"
      :dropzone="false"
      accept=".pdf,.docx,.xlsx,.md,.txt,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,text/plain,text/markdown"
    />
    <Teleport to="#app-route-actions">
      <div class="flex items-center gap-2">
        <UInput v-model="pageState.query" class="w-40 sm:w-56" icon="i-lucide-search" :placeholder="t('nav.search')" />
        <UDropdownMenu :items="createMenuItems" :content="{ align: 'end' }">
          <UButton icon="i-lucide-plus" :label="t('common.new')" class="hidden sm:inline-flex" />
          <UButton icon="i-lucide-plus" aria-label="New" class="sm:hidden" />
        </UDropdownMenu>
        <UButton
          icon="i-lucide-upload"
          :label="t('common.upload')"
          class="hidden sm:inline-flex"
          :loading="actionsState.uploadBusy"
          @click="fileUpload?.select()"
        />
        <UButton
          icon="i-lucide-upload"
          aria-label="Upload"
          class="sm:hidden"
          :loading="actionsState.uploadBusy"
          @click="fileUpload?.select()"
        />
        <UButton
          icon="i-lucide-refresh-cw"
          :label="t('common.refresh')"
          class="hidden sm:inline-flex"
          :disabled="treeState.treeLoading || pageState.loading"
          @click="refreshLibraryData"
        />
        <UButton
          icon="i-lucide-refresh-cw"
          aria-label="Refresh"
          class="sm:hidden"
          :disabled="treeState.treeLoading || pageState.loading"
          @click="refreshLibraryData"
        />
      </div>
    </Teleport>

    <UContextMenu :items="activeContextMenuItems">
      <section
        class="grid h-full min-h-0 gap-2 overflow-hidden rounded-lg bg-surface-0 dark:bg-surface-950"
        :class="treeState.breadcrumbItems.length > 0
          ? 'grid-rows-[auto_minmax(0,1fr)]'
          : 'grid-rows-[minmax(0,1fr)]'"
      >
        <LibraryToolbar
          v-if="treeState.breadcrumbItems.length > 0"
          :breadcrumb-home="treeState.breadcrumbHome"
          :breadcrumb-items="treeState.breadcrumbItems"
          :search-query="pageState.query"
          :show-search="false"
          @update:search-query="pageState.query = $event"
        />
        <LibraryResourceTable
          :create-folder-busy="actionsState.createFolderBusy"
          :create-source-folder-busy="sourceFolderState.busy"
          :entries="pageState.entries"
          :error="treeState.treeError || pageState.error"
          :first="pageState.first"
          :group-entries="visibleGroupEntries"
          :group-page="visibleChildGroupPage"
          hide-actions
          hide-group-paths
          compact
          :expanded-keys="treeState.expandedTreeKeys"
          :loading="treeState.treeLoading || pageState.loading"
          paginated
          :pagination="pageState.pagination"
          :page-size="pageState.pageSize"
          :resource-search-query="pageState.query"
          :retrying-file-ids="actionsState.retryingFileIds"
          :unavailable-file-ids="actionsState.unavailableFileIds"
          :selected-folder-ready="!!treeState.selectedFolder"
          :selection="treeState.selectedExplorerEntry"
          :sort-field="pageState.sortBy"
          :sort-order="pageState.sortOrder"
          :status-filter="pageState.statusFilter"
          :table-context-selection="treeState.resourceContextEntry"
          :upload-busy="actionsState.uploadBusy"
          :total-records="pageState.total"
          @update:selection="treeState.selectedExplorerEntry = $event"
          @update:tableContextSelection="treeState.resourceContextEntry = $event"
          @row-click="handleExplorerRowClick"
          @row-dblclick="handleExplorerRowDoubleClick"
          @row-contextmenu="handleExplorerRowContextMenu"
          @page="pageState.changePage($event.first, $event.rows)"
          @sort="pageState.changeSort($event.sortField, $event.sortOrder)"
          @status-filter="pageState.changeStatusFilter($event)"
          @group-contextmenu="handleGroupRowContextMenu"
          @surface-contextmenu="handleSurfaceContextMenu"
          @open-entry="openExplorerEntry"
          @move-entry="actionsState.moveExplorerEntry"
          @delete-entry="actionsState.deleteExplorerEntry"
          @open-group="emit('open-child-group', $event.group)"
          @group-page="emit('child-group-page', $event)"
          @group-page-size="emit('child-group-page-size', $event)"
          @edit-group="emit('edit-child-group', $event.group)"
          @move-group="emit('move-child-group', $event.group)"
          @delete-group="emit('delete-child-group', $event.group)"
          @toggle-folder="treeState.toggleFolderExpansion($event.id)"
          @refresh="treeState.refreshLibrary(detailState.loadDetail)"
          @retry="refreshLibraryData"
          @retry-entry="retryExplorerEntry"
          @create-folder="actionsState.openCreateFolderDialog()"
          @create-source-folder="sourceFolderState.openCreate()"
          @sync-source-folder="sourceFolderState.sync($event.id)"
          @upload-select="actionsState.handleFileSelection"
        />
      </section>
    </UContextMenu>

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
      :open="sourceFolderState.open"
      :busy="sourceFolderState.busy"
      :folder-name="sourceFolderState.folderName"
      :folder-name-readonly="!!sourceFolderState.folderId"
      :title="sourceFolderState.title"
      :value="sourceFolderState.value"
      @cancel="sourceFolderState.open = false"
      @confirm="sourceFolderState.save"
      @update:value="sourceFolderState.value = $event"
    />

    <UModal
      v-model:open="previewState.previewDialogVisible"

      :title="previewState.previewTitle"
      class="library-preview-dialog w-[min(96vw,72rem)] max-w-[min(96vw,72rem)]"
    >
      <template #body>
        <LibraryPreviewPanel
          :active-section-key="detailState.activeSectionKey"
          :detail="detailState.detail"
          :detail-loading="detailState.detailLoading"
          :group-path="groupPath"
          :selected-file-id="treeState.selectedFileId"
          :selected-folder-summary="treeState.selectedFolderSummary"
          :retrying="!!treeState.selectedFileId && actionsState.retryingFileIds.includes(treeState.selectedFileId)"
          @retry="actionsState.retryFile"
          @update:active-section-key="detailState.activeSectionKey = $event"
        />
      </template>
    </UModal>
  </div>
</template>
