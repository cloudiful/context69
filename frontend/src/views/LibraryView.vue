<script setup lang="ts">
import { computed, proxyRefs, watch } from "vue";
import type { ContextMenuItem } from "@nuxt/ui";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import LibraryCreateFolderDialog from "../components/LibraryCreateFolderDialog.vue";
import LibraryMoveDialog from "../components/LibraryMoveDialog.vue";
import LibraryPreviewPanel from "../components/LibraryPreviewPanel.vue";
import LibraryPreviewShell from "../components/LibraryPreviewShell.vue";
import LibraryResourceTable from "../components/LibraryResourceTable.vue";
import LibraryToolbar from "../components/LibraryToolbar.vue";
import { useLibraryActions } from "../composables/library/use-library-actions";
import { useLibraryDetail } from "../composables/library/use-library-detail";
import { useLibraryPreview } from "../composables/library/use-library-preview";
import { useLibraryPage } from "../composables/library/use-library-page";
import { useLibraryTree } from "../composables/library/use-library-tree";
import type { ExplorerEntry } from "../types/library";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const tree = useLibraryTree({
  route,
  router,
  t,
});
const treeState = proxyRefs(tree);

const page = useLibraryPage({
  folder: tree.selectedFolder,
  t,
});
const pageState = proxyRefs(page);

async function refreshLibraryData() {
  await tree.loadTree();
  await page.loadPage();
}

const detail = useLibraryDetail({
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
  loadTree: refreshLibraryData,
  moveOptions: tree.moveOptions,
  replaceQuery: tree.replaceQuery,
  selectFile: tree.selectFile,
  selectedFolder: tree.selectedFolder,
  selectedFileId: tree.selectedFileId,
  t,
  updateExpandedForFolder: tree.updateExpandedForFolder,
  previewDocked: preview.previewDocked,
  previewDialogVisible: preview.previewDialogVisible,
});
const actionsState = proxyRefs(actions);

const resourceMenuItems = computed<ContextMenuItem[]>(() => {
  const entry = treeState.resourceContextEntry;
  if (!entry) {
    return [];
  }

  if (entry.kind === "folder") {
    return [
      {
        label: t("library.openFolder"),
        icon: "i-lucide-folder-open",
        onSelect: () => {
          void treeState.selectFolder(entry.id);
        },
      },
      {
        label: t("library.newFolder"),
        icon: "i-lucide-folder-plus",
        onSelect: () => {
          actionsState.openCreateFolderDialog(entry.folder);
        },
      },
      {
        label: t("common.move"),
        icon: "i-lucide-folder-input",
        onSelect: () => {
          actionsState.openMoveFolderDialog(entry.folder);
        },
      },
      {
        label: t("common.delete"),
        icon: "i-lucide-trash-2",
        color: "error",
        onSelect: () => {
          void actionsState.deleteFolder(entry.folder);
        },
      },
    ];
  }

  return [
    {
      label: t("library.preview"),
      icon: "i-lucide-eye",
      onSelect: () => {
        void actionsState.revealPreviewForFile(entry.id);
      },
    },
    {
      label: t("common.move"),
      icon: "i-lucide-file-input",
      onSelect: () => {
        actionsState.openMoveFileDialog(entry.file);
      },
    },
    {
      label: t("common.delete"),
      icon: "i-lucide-trash-2",
      color: "error",
      onSelect: () => {
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
    return;
  }

  void actionsState.revealPreviewForFile(entry.id);
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
}

async function refreshLibrary() {
  await refreshLibraryData();
  await detailState.loadDetail(treeState.selectedFileId);
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

watch(page.entries, (entries) => {
  treeState.syncSelectedExplorerEntry(entries);
}, { immediate: true });

watch(tree.selectedFolderId, () => {
  page.reset();
  void page.loadPage();
});

watch(detail.detail, (nextDetail) => {
  const fileId = tree.selectedFileId.value;
  if (!fileId || !nextDetail || nextDetail.file_id !== fileId) return;
  if (nextDetail.folder_id !== tree.selectedFolderId.value) {
    void tree.replaceQuery(nextDetail.folder_id ?? null, fileId);
  }
});

watch(page.query, () => {
  pageState.page = 1;
  void page.loadPage();
});

void refreshLibraryData();

const explorerEntries = computed(() => page.entries.value);

defineExpose({
  explorerEntries,
  handleExplorerRowContextMenu,
  handleExplorerRowDoubleClick,
  resourceMenuItems,
});
</script>

<template>
  <div class="grid min-h-[calc(100vh-4.75rem)] gap-2">
    <LibraryToolbar
      :breadcrumb-home="treeState.breadcrumbHome"
      :breadcrumb-items="treeState.breadcrumbItems"
      :count-label="t('library.resourceCount', { count: pageState.total })"
      :search-query="pageState.query"
      @update:search-query="pageState.query = $event"
    />

    <UContextMenu :items="[resourceMenuItems]">
      <section
      class="h-auto min-h-[calc(100vh-8.5rem)] overflow-hidden rounded-lg bg-surface-0 dark:bg-surface-950 md:h-[calc(100dvh-var(--library-workspace-offset,9.25rem))] md:min-h-0"
    >
      <UDashboardGroup v-if="previewState.showDockedPreview" :persistent="false">
        <UDashboardPanel :default-size="62" :min-size="42" resizable>
          <LibraryResourceTable
            :create-folder-busy="actionsState.createFolderBusy"
            :entries="pageState.entries"
            :error="pageState.error"
            :first="pageState.first"
            :expanded-keys="treeState.expandedTreeKeys"
            :loading="treeState.treeLoading || pageState.loading"
            paginated
            :page-size="pageState.pageSize"
            :resource-search-query="pageState.query"
            :selected-folder-ready="!!treeState.selectedFolder"
            :selection="treeState.selectedExplorerEntry"
            :sort-field="pageState.sortBy"
            :sort-order="pageState.sortOrder"
            :status-filter="pageState.statusFilter"
            :table-context-selection="treeState.resourceContextEntry"
            :total-records="pageState.total"
            :upload-busy="actionsState.uploadBusy"
            @update:selection="treeState.selectedExplorerEntry = $event"
            @update:tableContextSelection="treeState.resourceContextEntry = $event"
            @row-click="handleExplorerRowClick"
            @row-dblclick="handleExplorerRowDoubleClick"
            @row-contextmenu="handleExplorerRowContextMenu"
            @page="pageState.changePage($event.first, $event.rows)"
            @sort="pageState.changeSort($event.sortField, $event.sortOrder)"
            @status-filter="pageState.changeStatusFilter($event)"
            @open-entry="openExplorerEntry"
            @move-entry="moveExplorerEntry"
            @delete-entry="deleteExplorerEntry"
            @toggle-folder="treeState.toggleFolderExpansion($event.id)"
            @refresh="refreshLibrary"
            @create-folder="actionsState.openCreateFolderDialog()"
            @upload-select="actionsState.handleFileSelection"
          />
        </UDashboardPanel>

        <UDashboardResizeHandle />
        <UDashboardPanel :default-size="38" :min-size="28" resizable>
          <LibraryPreviewShell
            :title="previewState.previewTitle"
            class="library-docked-preview border-l border-surface"
          >
            <LibraryPreviewPanel
              :active-section-key="detailState.activeSectionKey"
              :detail="detailState.detail"
              :detail-loading="detailState.detailLoading"
              :selected-file-id="treeState.selectedFileId"
              :selected-folder-summary="treeState.selectedFolderSummary"
              @update:active-section-key="detailState.activeSectionKey = $event"
            />
          </LibraryPreviewShell>
        </UDashboardPanel>
      </UDashboardGroup>

      <LibraryResourceTable
        v-else
        :create-folder-busy="actionsState.createFolderBusy"
        :entries="pageState.entries"
        :error="pageState.error"
        :first="pageState.first"
        :expanded-keys="treeState.expandedTreeKeys"
        :loading="treeState.treeLoading || pageState.loading"
        paginated
        :page-size="pageState.pageSize"
        :resource-search-query="pageState.query"
        :selected-folder-ready="!!treeState.selectedFolder"
        :selection="treeState.selectedExplorerEntry"
        :sort-field="pageState.sortBy"
        :sort-order="pageState.sortOrder"
        :status-filter="pageState.statusFilter"
        :table-context-selection="treeState.resourceContextEntry"
        :total-records="pageState.total"
        :upload-busy="actionsState.uploadBusy"
        @update:selection="treeState.selectedExplorerEntry = $event"
        @update:tableContextSelection="treeState.resourceContextEntry = $event"
        @row-click="handleExplorerRowClick"
        @row-dblclick="handleExplorerRowDoubleClick"
        @row-contextmenu="handleExplorerRowContextMenu"
        @page="pageState.changePage($event.first, $event.rows)"
        @sort="pageState.changeSort($event.sortField, $event.sortOrder)"
        @status-filter="pageState.changeStatusFilter($event)"
        @open-entry="openExplorerEntry"
        @move-entry="moveExplorerEntry"
        @delete-entry="deleteExplorerEntry"
        @toggle-folder="treeState.toggleFolderExpansion($event.id)"
        @refresh="refreshLibrary"
        @create-folder="actionsState.openCreateFolderDialog()"
        @upload-select="actionsState.handleFileSelection"
      />
      </section>
    </UContextMenu>

    <UModal
      v-model:open="previewState.previewDialogVisible"
      class="library-preview-dialog w-[min(96vw,72rem)] max-w-[min(96vw,72rem)]"
      :modal="true"
      :title="previewState.previewTitle"
    >
    <template #body>
<LibraryPreviewShell :title="previewState.previewTitle" :show-header="false">
        <LibraryPreviewPanel
          :active-section-key="detailState.activeSectionKey"
          :detail="detailState.detail"
          :detail-loading="detailState.detailLoading"
          :selected-file-id="treeState.selectedFileId"
          :selected-folder-summary="treeState.selectedFolderSummary"
          @update:active-section-key="detailState.activeSectionKey = $event"
        />
      </LibraryPreviewShell>
    </template>
    </UModal>

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
