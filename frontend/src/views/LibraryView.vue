<script setup lang="ts">
import { computed, proxyRefs, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import ContextMenu from "primevue/contextmenu";
import Dialog from "primevue/dialog";
import Message from "primevue/message";
import Splitter from "primevue/splitter";
import SplitterPanel from "primevue/splitterpanel";

import LibraryCreateFolderDialog from "../components/LibraryCreateFolderDialog.vue";
import LibraryMoveDialog from "../components/LibraryMoveDialog.vue";
import LibraryPreviewPanel from "../components/LibraryPreviewPanel.vue";
import LibraryPreviewShell from "../components/LibraryPreviewShell.vue";
import LibraryResourceTable from "../components/LibraryResourceTable.vue";
import LibraryToolbar from "../components/LibraryToolbar.vue";
import { useLibraryActions } from "../composables/library/use-library-actions";
import { useLibraryDetail } from "../composables/library/use-library-detail";
import { useLibraryPreview } from "../composables/library/use-library-preview";
import { useLibraryTree } from "../composables/library/use-library-tree";
import { createLibraryStatusHelpers } from "../utils/library-status";
import type { ExplorerEntry } from "../types/library";

interface LibraryMenuItem {
  command: () => void;
  icon: string;
  label: string;
}

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const { statusLabel } = createLibraryStatusHelpers();
const mapStatusLabel = (status: string) => statusLabel(status as "pending" | "running" | "succeeded" | "failed");

const tree = useLibraryTree({
  route,
  router,
  statusLabel: mapStatusLabel,
  t,
});
const treeState = proxyRefs(tree);

const detail = useLibraryDetail({
  loadTree: tree.loadTree,
  selectedFileId: tree.selectedFileId,
  t,
});
const detailState = proxyRefs(detail);

const preview = useLibraryPreview({
  detail: detail.detail,
  selectedFileId: tree.selectedFileId,
  selectedFolderSummary: tree.selectedFolderSummary,
  t,
});
const previewState = proxyRefs(preview);

const actions = useLibraryActions({
  loadTree: tree.loadTree,
  moveOptions: tree.moveOptions,
  replaceQuery: tree.replaceQuery,
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

const resourceMenuItems = computed<LibraryMenuItem[]>(() => {
  const entry = treeState.resourceContextEntry;
  if (!entry) {
    return [];
  }

  if (entry.kind === "folder") {
    return [
      {
        label: t("library.openFolder"),
        icon: "pi pi-folder-open",
        command: () => {
          void treeState.selectFolder(entry.id);
        },
      },
      {
        label: t("library.newFolder"),
        icon: "pi pi-folder-plus",
        command: () => {
          actionsState.openCreateFolderDialog(entry.folder);
        },
      },
      {
        label: t("common.move"),
        icon: "pi pi-arrows-alt",
        command: () => {
          actionsState.openMoveFolderDialog(entry.folder);
        },
      },
      {
        label: t("common.delete"),
        icon: "pi pi-trash",
        command: () => {
          void actionsState.deleteFolder(entry.folder);
        },
      },
    ];
  }

  return [
    {
      label: t("library.preview"),
      icon: "pi pi-eye",
      command: () => {
        void actionsState.revealPreviewForFile(entry.id);
      },
    },
    {
      label: t("common.move"),
      icon: "pi pi-arrows-alt",
      command: () => {
        actionsState.openMoveFileDialog(entry.file);
      },
    },
    {
      label: t("common.delete"),
      icon: "pi pi-trash",
      command: () => {
        void actionsState.deleteFile(entry.file);
      },
    },
  ];
});

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

function moveExplorerEntry(entry: ExplorerEntry) {
  actionsState.moveExplorerEntry(entry);
}

function deleteExplorerEntry(entry: ExplorerEntry) {
  actionsState.deleteExplorerEntry(entry);
}

function handleExplorerRowContextMenu(event: { originalEvent: Event; data: ExplorerEntry }) {
  treeState.resourceContextEntry = event.data;
  resourceContextMenu.value?.show(event.originalEvent);
}

async function refreshLibrary() {
  await treeState.refreshLibrary(detailState.loadDetail);
}

watch(
  tree.selectedFileId,
  (fileId) => {
    if (fileId && !previewState.previewDocked) {
      previewState.previewDialogVisible = true;
    }
    void detailState.loadDetail(fileId);
  },
  { immediate: true },
);

watch(
  tree.selectedFolderId,
  (folderId) => {
    treeState.updateExpandedForFolder(folderId);
  },
);

watch(
  tree.explorerEntries,
  (entries) => {
    treeState.syncSelectedExplorerEntry(entries);
  },
  { immediate: true },
);

void tree.loadTree();

const explorerEntries = computed(() => tree.explorerEntries.value);

defineExpose({
  explorerEntries,
  handleExplorerRowContextMenu,
  handleExplorerRowDoubleClick,
  resourceMenuItems,
});
</script>

<template>
  <div class="library-layout">
    <ContextMenu ref="resourceContextMenu" :model="resourceMenuItems" @hide="treeState.resourceContextEntry = null" />

    <LibraryToolbar
      :breadcrumb-home="treeState.breadcrumbHome"
      :breadcrumb-items="treeState.breadcrumbItems"
      :count-label="treeState.filteredResourceCountLabel"
      :search-query="treeState.resourceSearchQuery"
      @update:search-query="treeState.resourceSearchQuery = $event"
    />

    <Message v-if="treeState.treeError" severity="error" :closable="false">
      {{ treeState.treeError }}
    </Message>

    <section class="library-workspace" :class="{ 'library-workspace-docked': previewState.showDockedPreview }">
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
            @open-entry="openExplorerEntry"
            @move-entry="moveExplorerEntry"
            @delete-entry="deleteExplorerEntry"
            @toggle-folder="treeState.toggleFolderExpansion($event.id)"
            @refresh="refreshLibrary"
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
        @open-entry="openExplorerEntry"
        @move-entry="moveExplorerEntry"
        @delete-entry="deleteExplorerEntry"
        @toggle-folder="treeState.toggleFolderExpansion($event.id)"
        @refresh="refreshLibrary"
        @create-folder="actionsState.openCreateFolderDialog()"
        @upload-select="actionsState.handleFileSelection"
      />
    </section>

    <Dialog
      v-model:visible="previewState.previewDialogVisible"
      class="library-preview-dialog"
      :modal="true"
      :header="previewState.previewTitle"
      :style="{ width: 'min(96vw, 74rem)' }"
    >
      <LibraryPreviewShell :title="previewState.previewTitle" :show-header="false">
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
    </Dialog>

    <LibraryCreateFolderDialog
      :open="!!actionsState.createDialog"
      :busy="actionsState.createFolderBusy"
      :parent-name="actionsState.createDialog?.parentFolderName ?? t('library.rootFolder')"
      @cancel="actionsState.createDialog = null"
      @confirm="actionsState.confirmCreateFolder"
    />

    <LibraryMoveDialog
      :open="!!actionsState.moveDialog"
      :busy="actionsState.actionBusy"
      :title="actionsState.moveDialog ? t(actionsState.moveDialog.kind === 'folder' ? 'library.moveFolderTitle' : 'library.moveFileTitle', { name: actionsState.moveDialog.name }) : ''"
      :description="actionsState.moveDialog ? t(actionsState.moveDialog.kind === 'folder' ? 'library.moveFolderDescription' : 'library.moveFileDescription') : ''"
      :current-folder-id="actionsState.moveDialog?.currentFolderId ?? null"
      :options="actionsState.filteredMoveOptions"
      @cancel="actionsState.moveDialog = null"
      @confirm="actionsState.confirmMove"
    />
  </div>
</template>
