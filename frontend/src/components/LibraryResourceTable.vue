<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { DropdownMenuItem, TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import AppStateMessage from "./AppStateMessage.vue";
import { useLibraryResourceTable } from "../composables/library/use-library-resource-table";
import type { LibraryIngestStatus } from "../services/api";
import type { ExplorerEntry, GroupExplorerEntry, LibraryBrowserEntry } from "../types/library";
import { formatTimestamp } from "../utils/format";
import { createLibraryStatusHelpers } from "../utils/library-status";

const props = withDefaults(defineProps<{
  createFolderBusy: boolean;
  createSourceFolderBusy?: boolean;
  compact?: boolean;
  entries: ExplorerEntry[];
  error?: string | null;
  first?: number;
  groupEntries?: GroupExplorerEntry[];
  hideActions?: boolean;
  hideGroupPaths?: boolean;
  expandedKeys: Record<string, boolean>;
  loading: boolean;
  pageSize?: number;
  paginated?: boolean;
  resourceSearchQuery: string;
  retryingFileIds?: string[];
  unavailableFileIds?: string[];
  selectedFolderReady: boolean;
  selection: ExplorerEntry | null;
  sortField?: string;
  sortOrder?: number;
  statusFilter?: LibraryIngestStatus | null;
  tableContextSelection: ExplorerEntry | null;
  uploadBusy: boolean;
  totalRecords?: number;
}>(), {
  groupEntries: () => [], hideActions: false, hideGroupPaths: false, compact: false,
  first: 0, pageSize: 50, paginated: false, sortField: "updated_at", sortOrder: -1,
  statusFilter: null, totalRecords: 0, retryingFileIds: () => [], unavailableFileIds: () => [],
});

const emit = defineEmits<{
  "create-folder": []; "create-source-folder": []; "delete-group": [GroupExplorerEntry];
  "edit-group": [GroupExplorerEntry]; "group-contextmenu": [{ originalEvent: Event; data: GroupExplorerEntry }];
  "move-group": [GroupExplorerEntry]; page: [{ first: number; rows: number }]; "open-group": [GroupExplorerEntry];
  "open-entry": [ExplorerEntry]; refresh: []; retry: []; "retry-entry": [ExplorerEntry];
  sort: [{ sortField: "name" | "type" | "status" | "size" | "updated_at"; sortOrder: number }];
  "status-filter": [LibraryIngestStatus | null]; "row-click": [{ data: ExplorerEntry }];
  "row-contextmenu": [{ originalEvent: Event; data: ExplorerEntry }]; "row-dblclick": [{ data: ExplorerEntry }];
  "toggle-folder": [ExplorerEntry]; "move-entry": [ExplorerEntry]; "delete-entry": [ExplorerEntry];
  "sync-source-folder": [ExplorerEntry]; "surface-contextmenu": [{ originalEvent: MouseEvent }];
  "update:selection": [ExplorerEntry | null]; "update:tableContextSelection": [ExplorerEntry | null];
  "upload-select": [event: { files?: File[] }];
}>();

const { t } = useI18n();
const { statusLabel, statusSeverity } = createLibraryStatusHelpers();
const table = useLibraryResourceTable(props, t, statusLabel);
const uploadFiles = ref<File[] | null>(null);
const sorting = ref([{ id: props.sortField, desc: props.sortOrder !== 1 }]);
const currentPage = computed(() => Math.floor(props.first / props.pageSize) + 1);
const columns = computed<TableColumn<LibraryBrowserEntry>[]>(() => [
  { accessorKey: "name", header: t("library.filename") },
  { id: "type", header: t("library.typeLabel") },
  { id: "status", header: t("library.statusLabel") },
  { id: "size", header: t("library.sizeLabel") },
  { id: "updated_at", header: t("library.updatedColumn") },
  ...(!props.hideActions ? [{ id: "actions", header: t("sources.table.action") }] : []),
]);

watch(uploadFiles, (files) => {
  if (!files?.length) return;
  emit("upload-select", { files });
  uploadFiles.value = null;
});
watch(() => [props.sortField, props.sortOrder] as const, ([field, order]) => {
  sorting.value = [{ id: field, desc: order !== 1 }];
});
watch(sorting, (value) => {
  const next = value[0];
  if (!next || !["name", "type", "status", "size", "updated_at"].includes(next.id)) return;
  const order = next.desc ? -1 : 1;
  if (next.id !== props.sortField || order !== props.sortOrder) {
    emit("sort", { sortField: next.id as "name" | "type" | "status" | "size" | "updated_at", sortOrder: order });
  }
}, { deep: true });

function openEntry(entry: LibraryBrowserEntry) {
  entry.kind === "group" ? emit("open-group", entry) : emit("open-entry", entry);
}
function selectEntry(_event: Event, row: { original: LibraryBrowserEntry }) {
  const entry = row.original;
  if (entry.kind === "group") return emit("open-group", entry);
  emit("update:selection", entry);
  emit("row-click", { data: entry });
}
function openEntryFromDoubleClick(entry: LibraryBrowserEntry) {
  entry.kind === "group" ? emit("open-group", entry) : emit("row-dblclick", { data: entry });
}
function contextEntry(event: Event, row: { original: LibraryBrowserEntry }) {
  const entry = row.original;
  if (entry.kind === "group") emit("group-contextmenu", { originalEvent: event, data: entry });
  else emit("row-contextmenu", { originalEvent: event, data: entry });
}
function moveEntry(entry: LibraryBrowserEntry) {
  entry.kind === "group" ? emit("move-group", entry) : emit("move-entry", entry);
}
function deleteEntry(entry: LibraryBrowserEntry) {
  entry.kind === "group" ? emit("delete-group", entry) : emit("delete-entry", entry);
}
function rowActions(entry: LibraryBrowserEntry): DropdownMenuItem[][] {
  const items: DropdownMenuItem[] = [{ label: t("common.open"), icon: "i-lucide-folder-open", onSelect: () => openEntry(entry) }];
  if (entry.kind === "group") items.push({ label: t("common.edit"), icon: "i-lucide-pencil", onSelect: () => emit("edit-group", entry) });
  if (entry.kind === "folder" && entry.isSourceFolder) items.push({ label: t("sources.sync"), icon: "i-lucide-refresh-cw", onSelect: () => emit("sync-source-folder", entry) });
  if (table.canMoveEntry(entry)) items.push({ label: t("common.move"), icon: "i-lucide-folder-input", onSelect: () => moveEntry(entry) });
  if (table.canDeleteEntry(entry)) items.push({ label: t("common.delete"), icon: "i-lucide-trash-2", color: "error", onSelect: () => deleteEntry(entry) });
  return [items];
}
function handleSurfaceContextMenu(event: MouseEvent) {
  if (!(event.target instanceof HTMLElement) || event.target.closest("tr")) return;
  emit("surface-contextmenu", { originalEvent: event });
}
</script>

<template>
  <div class="library-pane flex h-full min-h-0 flex-col gap-2 px-2 py-2">
    <div v-if="!props.hideActions" class="flex flex-wrap items-center justify-end gap-2">
      <UButton id="library-open-create-folder" color="neutral" variant="outline" :disabled="props.createFolderBusy || !props.selectedFolderReady" :label="t('library.newFolder')" @click="emit('create-folder')" />
      <UButton color="neutral" variant="outline" :disabled="props.createSourceFolderBusy || !props.selectedFolderReady" :label="t('library.newSourceFolder')" @click="emit('create-source-folder')" />
      <UFileUpload v-model="uploadFiles" multiple :preview="false" :dropzone="false" :disabled="props.uploadBusy || !props.selectedFolderReady" accept=".pdf,.docx,.xlsx,.md,.txt" class="w-auto">
        <template #default="{ open }"><UButton icon="i-lucide-upload" :loading="props.uploadBusy" :label="t('common.upload')" @click="open()" /></template>
      </UFileUpload>
    </div>

    <div v-if="props.paginated || table.statusOptions.value.length" class="flex flex-wrap items-center justify-between gap-2">
      <USelect :model-value="props.statusFilter" :items="table.statusOptions.value" value-key="value" class="w-48" :aria-label="t('library.filterByStatus')" @update:model-value="emit('status-filter', $event as LibraryIngestStatus | null)" />
      <UButton color="neutral" variant="ghost" icon="i-lucide-refresh-cw" :label="t('common.refresh')" @click="emit('refresh')" />
    </div>

    <div class="flex min-h-0 flex-1 flex-col overflow-auto" @contextmenu.self.prevent="handleSurfaceContextMenu">
      <AsyncStateBlock :error="props.error" :loading="props.loading" :loading-title="t('common.loading')" :loading-message="t('library.loadingFiles')">
        <template #error>
          <div class="grid justify-items-center gap-3 py-8 text-center">
            <AppStateMessage color="error" :title="t('common.error')">{{ props.error }}</AppStateMessage>
            <UButton size="sm" :label="t('common.retry')" :aria-label="t('common.retry')" @click="emit('retry')" />
          </div>
        </template>

        <UTable v-model:sorting="sorting" class="min-w-[52rem]" :data="table.displayEntries.value" :columns="columns" :loading="props.loading" :sorting-options="{ manualSorting: props.paginated }" @select="selectEntry" @contextmenu="contextEntry">
          <template #empty><div class="py-8 text-center text-sm text-muted">{{ table.hasActiveResourceFilter.value ? t("library.noMatchingResources") : t("library.emptyFolderMessage") }}</div></template>
          <template #name-cell="{ row }">
            <div class="flex min-w-0 items-start gap-1.5" :style="table.entryIndentStyle(row.original)">
              <UButton v-if="row.original.kind === 'folder'" class="library-folder-toggle" color="neutral" variant="ghost" size="xs" :icon="table.isFolderExpanded(row.original) ? 'i-lucide-chevron-down' : 'i-lucide-chevron-right'" :aria-label="row.original.name" @click.stop="emit('toggle-folder', row.original)" />
              <UIcon v-else :name="row.original.kind === 'group' ? 'i-lucide-users' : 'i-lucide-file'" class="mt-1 size-4 shrink-0 text-muted" />
              <button class="min-w-0 flex-1 text-left" type="button" :data-entry-key="row.original.key" @click.stop="openEntry(row.original)" @dblclick.stop="openEntryFromDoubleClick(row.original)">
                <span class="block truncate text-sm font-semibold">{{ row.original.name }}</span>
                <span v-if="row.original.kind === 'folder'" class="text-xs text-muted">{{ t("library.treeCounts", { folders: row.original.childFolderCount, files: row.original.fileCount }) }}</span>
                <span v-else-if="row.original.kind === 'group' && !props.hideGroupPaths" class="text-xs text-muted">{{ row.original.path }}</span>
              </button>
            </div>
          </template>
          <template #type-cell="{ row }"><span class="text-sm text-muted">{{ table.resourceTypeLabel(row.original) }}</span></template>
          <template #status-cell="{ row }">
            <UBadge v-if="row.original.kind === 'group'" :label="row.original.visibility" color="neutral" variant="subtle" />
            <span v-else-if="row.original.kind === 'file'" class="inline-flex items-center gap-1.5" :title="table.statusTooltip(row.original)">
              <UBadge :label="statusLabel(row.original.ingestStatus)" :color="statusSeverity(row.original.ingestStatus)" variant="subtle" />
              <UButton v-if="row.original.ingestStatus === 'failed' && !props.unavailableFileIds.includes(row.original.id)" color="neutral" variant="ghost" size="xs" icon="i-lucide-refresh-cw" :loading="table.isRetrying(row.original)" :aria-label="t('common.retry')" @click.stop="emit('retry-entry', row.original)" />
            </span>
            <span v-else class="text-sm text-muted">{{ table.resourceStatusLabel(row.original) }}</span>
          </template>
          <template #size-cell="{ row }"><span class="tabular-nums text-sm text-muted">{{ table.resourceSizeLabel(row.original) }}</span></template>
          <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ row.original.updatedAt ? formatTimestamp(row.original.updatedAt) : "-" }}</span></template>
          <template #actions-cell="{ row }"><UDropdownMenu :items="rowActions(row.original)"><UButton color="neutral" variant="ghost" icon="i-lucide-ellipsis" :aria-label="t('common.actions')" /></UDropdownMenu></template>
        </UTable>
      </AsyncStateBlock>
    </div>

    <UPagination v-if="props.paginated && props.totalRecords > props.pageSize" :page="currentPage" :items-per-page="props.pageSize" :total="props.totalRecords" class="justify-end" @update:page="emit('page', { first: ($event - 1) * props.pageSize, rows: props.pageSize })" />
  </div>
</template>
