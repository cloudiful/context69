<script setup lang="ts">
import { computed, onBeforeUnmount, proxyRefs, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

import LibraryResourceTable from "./LibraryResourceTable.vue";
import LibraryToolbar from "./LibraryToolbar.vue";
import ProjectFilesFileUpload, { type ProjectFilesFileUploadHandle } from "./ProjectFilesFileUpload.vue";
import ProjectFilesLibraryDialogs from "./ProjectFilesLibraryDialogs.vue";
import ProjectFilesPreviewModal from "./ProjectFilesPreviewModal.vue";
import ProjectFilesRouteActions from "./ProjectFilesRouteActions.vue";
import ProjectFilesScopedSearchModal from "./ProjectFilesScopedSearchModal.vue";
import { useProjectLibraryActions } from "../composables/project-library/use-project-library-actions";
import { useProjectFilesContextMenu } from "../composables/project-library/use-project-files-context-menu";
import { useProjectFilesDialogs } from "../composables/project-library/use-project-files-dialogs";
import { useProjectFilesRowHandlers } from "../composables/project-library/use-project-files-row-handlers";
import { useLibraryRetryAllFailed } from "../composables/project-library/use-library-retry-all-failed";
import { useProjectLibraryDetail } from "../composables/project-library/use-project-library-detail";
import { useProjectLibraryPage } from "../composables/project-library/use-project-library-page";
import { useScopedContentSearch } from "../composables/project-library/use-scoped-content-search";
import { useGroupBrowserEntries } from "../composables/project-library/use-group-browser-entries";
import { useLibraryPreview as useProjectLibraryPreview } from "../composables/library/use-library-preview";
import { useProjectLibraryTree } from "../composables/project-library/use-project-library-tree";
import { useProjectSourceFolder } from "../composables/project-library/use-project-source-folder";
import {
  buildLibraryFolderQuery,
  getParentFolderId,
  isValidLibraryFolder as isValidFolder,
  readLibraryFolderQuery,
} from "../composables/project-library/use-project-library-navigation";
import type { GroupPageResponse, GroupResponse } from "../services/api";
import { createLibraryStatusHelpers } from "../utils/library-status";

const props = defineProps<{
  childGroups: GroupResponse[];
  childGroupPage: GroupPageResponse;
  childGroupSearch: string;
  groupPath: string;
  groupName?: string | null;
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

function openFilePicker() {
  fileUpload.value?.trigger();
}

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { statusLabel } = createLibraryStatusHelpers();
const mapStatusLabel = (status: string) => statusLabel(status as "succeeded" | "failed");

async function requestFolderNavigation(folderId: string | null) {
  const currentQueryFolder = readLibraryFolderQuery(route.query);
  const currentSelected = tree.selectedFolderId.value;
  const hasFile = typeof route.query.file === "string" && !!route.query.file;
  if (folderId === currentSelected && (currentQueryFolder ?? null) === (folderId ?? null) && !hasFile) {
    return;
  }
  const nextQuery = buildLibraryFolderQuery(route.query, folderId, { clearFile: true });
  await router.push({
    name: "group-overview",
    params: { groupPath: props.groupPath },
    query: nextQuery,
  });
}

const tree = useProjectLibraryTree({
  groupPath: () => props.groupPath,
  statusLabel: mapStatusLabel,
  t,
  onNavigate: (folderId) => requestFolderNavigation(folderId),
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
  await detail.loadDetail(tree.selectedFileId.value);
}
const detail = useProjectLibraryDetail({
  groupPath: () => props.groupPath,
  t,
});
const detailState = proxyRefs(detail);
const scopedSearch = proxyRefs(useScopedContentSearch({
  groupPath: () => props.groupPath,
  folderPath: () => tree.selectedFolder.value?.path ?? null,
  t,
}));
const sourceFolderState = proxyRefs(useProjectSourceFolder({
  groupPath: () => props.groupPath,
  selectedFolder: tree.selectedFolder,
  refreshLibrary: refreshLibraryData,
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
const dialogs = proxyRefs(useProjectFilesDialogs({ actions: actionsState, sourceFolder: sourceFolderState, t }));
const retryAll = useLibraryRetryAllFailed({
  groupPath: () => props.groupPath,
  folderId: () => tree.selectedFolder.value?.folder_id ?? null,
  refresh: refreshLibraryData,
  isSourceUnavailable: (fileId: string) => actionsState.unavailableFileIds.includes(fileId),
  observeSourceAvailability: (detailEntry) => actionsState.observeSourceAvailability(detailEntry),
  t,
});
const retryAllState = proxyRefs(retryAll);
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
const selectedFileEntry = computed(() => treeState.selectedExplorerEntry?.kind === "file" ? treeState.selectedExplorerEntry : null);

const breadcrumbHome = computed(() => ({
  label: props.groupName ?? t("library.rootFolder"),
  onSelect: () => { void requestFolderNavigation(null); },
}));
const breadcrumbItems = computed(() => tree.selectedFolderTrail.value.slice(1).map((folder) => ({
  label: folder.name,
  onSelect: () => { void requestFolderNavigation(folder.folder_id ?? null); },
})));
const canGoUp = computed(() => tree.selectedFolderTrail.value.length > 1);

function handleToolbarUp() {
  const parentId = getParentFolderId(tree.selectedFolderTrail.value);
  if (tree.selectedFolderTrail.value.length < 2) return;
  void requestFolderNavigation(parentId);
}

const currentFolderName = computed(() => {
  if (!tree.selectedFolderId.value) {
    return t("library.rootFolder");
  }
  return tree.selectedFolderSummary.value?.name ?? props.groupName ?? t("library.rootFolder");
});
const scopedSearchPlaceholder = computed(() => t("search.scoped.placeholder", { folder: currentFolderName.value }));
const scopedSearchTitle = computed(() => t("search.scoped.title", { folder: currentFolderName.value }));

function runScopedSearch() {
  if (!scopedSearch.query.trim()) return;
  void scopedSearch.run();
}

const fileUpload = ref<ProjectFilesFileUploadHandle | null>(null);
const {
  groupContextEntry,
  handleExplorerRowClick,
  handleExplorerRowDoubleClick,
  handleExplorerRowContextMenu,
  handleGroupRowContextMenu,
  handleReleaseSource,
  handleSurfaceContextMenu,
  openExplorerEntry,
  retryExplorerEntry,
} = useProjectFilesRowHandlers({
  tree: treeState,
  actions: actionsState,
  sourceFolder: sourceFolderState,
});
const { activeContextMenuItems, createMenuItems } = useProjectFilesContextMenu({
  t,
  groupContextEntry,
  resourceContextEntry: () => treeState.resourceContextEntry,
  actions: actionsState,
  sourceFolder: sourceFolderState,
  selectFolder: (id) => { void treeState.selectFolder(id); },
  open: openExplorerEntry,
  releaseSource: handleReleaseSource,
  refresh: () => { void refreshLibraryData(); },
  upload: openFilePicker,
  emit,
});

function releaseSelectedFile(fileId: string) {
  void actionsState.releaseFileSource(fileId, detailState.detail?.filename ?? "");
}

let routeGeneration = 0;
let treeReadyPromise: Promise<void> | null = null;
let treeReadyGroup: string | null = null;

async function ensureTreeLoaded(): Promise<void> {
  if (tree.tree.value) return;
  if (treeReadyPromise && treeReadyGroup === props.groupPath) {
    await treeReadyPromise;
    return;
  }
  treeReadyGroup = props.groupPath;
  treeReadyPromise = tree.loadTree();
  try {
    await treeReadyPromise;
  } finally {
    treeReadyPromise = null;
  }
}

function readFileQuery(): string | null {
  const raw = route.query.file;
  if (typeof raw === "string" && raw) return raw;
  if (Array.isArray(raw) && typeof raw[0] === "string" && raw[0]) return raw[0];
  return null;
}

async function applyRouteToSelection(generation: number): Promise<void> {
  await ensureTreeLoaded();
  if (generation !== routeGeneration) return;
  const root = tree.tree.value?.root ?? null;
  if (!root) return;
  const folderQuery = readLibraryFolderQuery(route.query);
  const fileQuery = readFileQuery();

  if (fileQuery) {
    if (fileQuery !== tree.selectedFileId.value) {
      await tree.replaceSelection(tree.selectedFolderId.value ?? folderQuery ?? null, fileQuery);
      return;
    }
    const detailData = detail.detail.value;
    if (detailData && detailData.file_id === fileQuery) {
      const canonicalFolder = detailData.folder_id ?? null;
      if (tree.selectedFolderId.value !== canonicalFolder) {
        await tree.replaceSelection(canonicalFolder, fileQuery);
        tree.updateExpandedForFolder(canonicalFolder);
      } else {
        tree.updateExpandedForFolder(canonicalFolder);
      }
      if ((folderQuery ?? null) !== (canonicalFolder ?? null)) {
        if (generation !== routeGeneration) return;
        const nextQuery = { ...route.query };
        if (canonicalFolder === null) {
          delete nextQuery.folder;
        } else {
          nextQuery.folder = canonicalFolder;
        }
        await router.replace({ query: nextQuery });
      }
      return;
    }
    return;
  }

  if (!isValidFolder(root, folderQuery)) {
    if (folderQuery !== null) {
      await tree.replaceSelection(null, null);
      tree.updateExpandedForFolder(null);
      if (generation !== routeGeneration) return;
      const fresh = { ...route.query };
      delete fresh.folder;
      await router.replace({ query: fresh });
      return;
    }
    if (tree.selectedFolderId.value !== null || tree.selectedFileId.value !== null) {
      await tree.replaceSelection(null, null);
      tree.updateExpandedForFolder(null);
    }
    return;
  }

  if ((tree.selectedFolderId.value ?? null) !== (folderQuery ?? null)) {
    await tree.replaceSelection(folderQuery, null);
    tree.updateExpandedForFolder(folderQuery);
  } else if (tree.selectedFileId.value !== null && !fileQuery) {
    await tree.replaceSelection(folderQuery, null);
  }
}

watch(tree.selectedFileId, (fileId) => {
  if (fileId && !previewState.previewDocked) {
    previewState.previewDialogVisible = true;
  }
  void detailState.loadDetail(fileId);
}, { immediate: true });

watch(tree.selectedFolder, () => {
  treeState.updateExpandedForFolder(tree.selectedFolderId.value);
  page.reset();
  void page.loadPage();
  void retryAllState.loadFailedCount();
});

watch(detail.detail, (nextDetail) => {
  if (nextDetail) actionsState.observeSourceAvailability(nextDetail);
  const fileId = tree.selectedFileId.value;
  if (!fileId || !nextDetail || nextDetail.file_id !== fileId) return;
  if (nextDetail.folder_id !== tree.selectedFolderId.value) {
    void tree.replaceSelection(nextDetail.folder_id ?? null, fileId);
  }
});

watch(page.entries, (entries) => {
  treeState.syncSelectedExplorerEntry(entries);
}, { immediate: true });

watch(() => props.groupPath, async (next, prev) => {
  if (next === prev) return;
  routeGeneration += 1;
  const generation = routeGeneration;
  tree.resetTree();
  page.reset();
  treeReadyPromise = null;
  treeReadyGroup = null;
  await ensureTreeLoaded();
  if (generation !== routeGeneration) return;
  await applyRouteToSelection(generation);
}, { immediate: true });

watch(() => route.query.folder, async () => {
  const generation = routeGeneration;
  await applyRouteToSelection(generation);
});

watch(() => route.query.file, async () => {
  const generation = routeGeneration;
  await applyRouteToSelection(generation);
});

watch(tree.selectedFileId, (fileId) => {
  const current = readFileQuery();
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

watch(tree.selectedFolderId, (folderId) => {
  const queryFolder = readLibraryFolderQuery(route.query);
  if ((queryFolder ?? null) === (folderId ?? null)) return;
  if (!treeState.tree) return;
  const next = { ...route.query };
  if (folderId === null) {
    delete next.folder;
  } else {
    next.folder = folderId;
  }
  void router.replace({ query: next });
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
  actionsState.dispose();
  retryAllState.dispose();
  sourceFolderState.dispose();
  detail.dispose();
});
</script>

<template>
  <div class="project-files-panel h-full min-h-0">
    <ProjectFilesFileUpload ref="fileUpload" @select="actionsState.handleFileSelection" />
    <ProjectFilesRouteActions
      :create-menu-items="createMenuItems"
      :delete-source-after-processing="actionsState.deleteSourceAfterProcessing"
      :query="pageState.query"
      :retry-all-busy="retryAllState.retryAllBusy"
      :retry-all-failed-count="retryAllState.retryAllFailedCount"
      :scoped-query="scopedSearch.query"
      :scoped-search-loading="scopedSearch.loading"
      :scoped-search-placeholder="scopedSearchPlaceholder"
      :upload-busy="actionsState.uploadBusy"
      @retry-all-failed="retryAllState.retryAllFailed()"
      @run-scoped-search="runScopedSearch"
      @update:delete-source-after-processing="actionsState.deleteSourceAfterProcessing = $event"
      @update:query="pageState.query = $event"
      @update:scoped-query="scopedSearch.query = $event"
      @upload="openFilePicker"
    />

    <UContextMenu :items="activeContextMenuItems">
      <section
        class="grid h-full min-h-0 gap-2 overflow-hidden rounded-lg bg-surface-0 dark:bg-surface-950 grid-rows-[auto_minmax(0,1fr)]"
      >
        <LibraryToolbar
          :breadcrumb-home="breadcrumbHome"
          :breadcrumb-items="breadcrumbItems"
          :can-go-up="canGoUp"
          :search-query="pageState.query"
          :show-search="false"
          @update:search-query="pageState.query = $event"
          @up="handleToolbarUp"
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
          :releasing-file-ids="actionsState.releasingFileIds"
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
          @refresh="refreshLibraryData"
          @retry="refreshLibraryData"
          @retry-entry="retryExplorerEntry"
          @release-source="handleReleaseSource"
          @create-folder="actionsState.openCreateFolderDialog()"
          @create-source-folder="sourceFolderState.openCreate()"
          @sync-source-folder="sourceFolderState.sync($event.id)"
          @upload-select="actionsState.handleFileSelection"
        />
      </section>
    </UContextMenu>

    <ProjectFilesLibraryDialogs
      v-bind="dialogs.bindings"
      @create-folder-cancel="dialogs.cancelCreateFolder"
      @create-folder-confirm="dialogs.confirmCreateFolder"
      @create-text-file-cancel="dialogs.cancelCreateTextFile"
      @create-text-file-confirm="dialogs.confirmCreateTextFile"
      @move-cancel="dialogs.cancelMove"
      @move-confirm="dialogs.confirmMove"
      @source-folder-cancel="dialogs.cancelSourceFolder"
      @source-folder-confirm="dialogs.confirmSourceFolder"
      @source-folder-update-value="dialogs.updateSourceFolderValue"
    />

    <ProjectFilesPreviewModal
      :open="previewState.previewDialogVisible"
      :title="previewState.previewTitle"
      :active-section-key="detailState.activeSectionKey"
      :detail="detailState.detail"
      :detail-loading="detailState.detailLoading"
      :group-path="groupPath"
      :selected-file-id="treeState.selectedFileId"
      :selected-folder-summary="treeState.selectedFolderSummary"
      :releasable="selectedFileEntry ? actionsState.canReleaseSource(selectedFileEntry) : false"
      :releasing="!!treeState.selectedFileId && actionsState.releasingFileIds.includes(treeState.selectedFileId)"
      :retrying="!!treeState.selectedFileId && actionsState.retryingFileIds.includes(treeState.selectedFileId)"
      @retry="actionsState.retryFile"
      @release="releaseSelectedFile"
      @update:active-section-key="detailState.activeSectionKey = $event"
      @update:open="previewState.previewDialogVisible = $event"
    />

    <ProjectFilesScopedSearchModal
      :open="scopedSearch.modalVisible"
      :title="scopedSearchTitle"
      :loading="scopedSearch.loading"
      :query="scopedSearch.query"
      :results="scopedSearch.results"
      @update:open="scopedSearch.modalVisible = $event"
    />
  </div>
</template>
