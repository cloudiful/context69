<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable, { type DataTableFilterEvent, type DataTableOperatorFilterMetaData } from "primevue/datatable";
import Paginator from "primevue/paginator";
import Select from "primevue/select";
import Tag from "./AppTag.vue";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import AppStateMessage from "./AppStateMessage.vue";
import LibraryResourceCards from "./LibraryResourceCards.vue";
import type { ExplorerEntry, GroupExplorerEntry, LibraryBrowserEntry } from "../types/library";
import type { LibraryIngestStatus } from "../services/api";
import { toolPrimaryButtonClass } from "../ui/button-classes";
import { createLibraryStatusHelpers } from "../utils/library-status";
import { formatBytes, formatTimestamp } from "../utils/format";

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
  groupEntries: () => [],
  hideActions: false,
  hideGroupPaths: false,
  compact: false,
  first: 0,
  pageSize: 50,
  paginated: false,
  sortField: "updated_at",
  sortOrder: -1,
  statusFilter: null,
  totalRecords: 0,
  retryingFileIds: () => [],
  unavailableFileIds: () => [],
});

const emit = defineEmits<{
  "create-folder": [];
  "create-source-folder": [];
  "delete-group": [GroupExplorerEntry];
  "edit-group": [GroupExplorerEntry];
  "group-contextmenu": [{ originalEvent: Event; data: GroupExplorerEntry }];
  "move-group": [GroupExplorerEntry];
  page: [{ first: number; rows: number }];
  "open-group": [GroupExplorerEntry];
  "open-entry": [ExplorerEntry];
  refresh: [];
  retry: [];
  "retry-entry": [ExplorerEntry];
  sort: [{ sortField: "name" | "type" | "status" | "size" | "updated_at"; sortOrder: number }];
  "status-filter": [LibraryIngestStatus | null];
  "row-click": [{ data: ExplorerEntry }];
  "row-contextmenu": [{ originalEvent: Event; data: ExplorerEntry }];
  "row-dblclick": [{ data: ExplorerEntry }];
  "toggle-folder": [ExplorerEntry];
  "move-entry": [ExplorerEntry];
  "delete-entry": [ExplorerEntry];
  "sync-source-folder": [ExplorerEntry];
  "surface-contextmenu": [{ originalEvent: MouseEvent }];
  "update:selection": [ExplorerEntry | null];
  "update:tableContextSelection": [ExplorerEntry | null];
  "upload-select": [event: { files?: File[] }];
}>();

const { t } = useI18n();
const { statusLabel, statusSeverity } = createLibraryStatusHelpers();
const displayEntries = computed<LibraryBrowserEntry[]>(() => [...props.groupEntries, ...props.entries]);
const hasActiveResourceFilter = computed(() => !!props.resourceSearchQuery || !!props.statusFilter);
const filters = ref({
  status: {
    operator: "and",
    constraints: [{ value: props.statusFilter, matchMode: "equals" }],
  },
});
const statusOptions = computed(() => [
  { label: t("library.allStatuses"), value: null },
  ...(["pending", "running", "succeeded", "failed"] as const).map((value) => ({
    label: statusLabel(value),
    value,
  })),
]);
const tableStateKey = computed(() => props.compact
  ? "context69:table:group-library:v5"
  : "context69:table:library:v3");

function resourceTypeLabel(entry: LibraryBrowserEntry): string {
  if (entry.kind === "group") return "groups.groupType";
  return entry.kind === "folder" ? "library.folderType" : "library.fileType";
}

function resourceStatusLabel(entry: LibraryBrowserEntry): string {
  if (entry.kind === "group") return entry.visibility;
  if (entry.kind === "folder") {
    return entry.processingCount > 0
      ? `library.folderProcessing`
      : "—";
  }

  return statusLabel(entry.ingestStatus);
}

function resourceSizeLabel(entry: LibraryBrowserEntry): string {
  if (entry.kind !== "file") {
    return "—";
  }

  return formatBytes(entry.sizeBytes);
}

function statusTooltip(entry: LibraryBrowserEntry): string | undefined {
  if (entry.kind !== "file" || entry.ingestStatus !== "failed") {
    return undefined;
  }

  return entry.errorMessage || undefined;
}

function isRetrying(entry: LibraryBrowserEntry): boolean {
  return entry.kind === "file" && props.retryingFileIds.includes(entry.id);
}

function entryIndentStyle(entry: LibraryBrowserEntry) {
  return {
    "--library-entry-depth": String(entry.depth),
  };
}

function isFolderExpanded(entry: LibraryBrowserEntry): boolean {
  return entry.kind === "folder" && !!props.expandedKeys[entry.id ?? "__root__"];
}

function handleSurfaceContextMenu(event: MouseEvent) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) {
    return;
  }

  if (target.closest("tr") || target.closest("[data-library-card]")) {
    return;
  }

  emit("surface-contextmenu", { originalEvent: event });
}

function canMoveEntry(entry: LibraryBrowserEntry): boolean {
  if (entry.kind === "group") return true;
  if (entry.kind === "folder") {
    return !entry.isSourceRecordsFolder;
  }
  return !entry.isSourceConfigFile && !entry.isSourceRecordFile;
}

function canDeleteEntry(entry: LibraryBrowserEntry): boolean {
  if (entry.kind === "group") return true;
  if (entry.kind === "folder") {
    return !entry.isSourceRecordsFolder;
  }
  return !entry.isSourceConfigFile && !entry.isSourceRecordFile;
}

function openEntry(entry: LibraryBrowserEntry) {
  if (entry.kind === "group") {
    emit("open-group", entry);
    return;
  }
  emit("open-entry", entry);
}

function moveEntry(entry: LibraryBrowserEntry) {
  if (entry.kind === "group") {
    emit("move-group", entry);
    return;
  }
  emit("move-entry", entry);
}

function deleteEntry(entry: LibraryBrowserEntry) {
  if (entry.kind === "group") {
    emit("delete-group", entry);
    return;
  }
  emit("delete-entry", entry);
}

function handleRowClick(event: { data: LibraryBrowserEntry }) {
  if (event.data.kind === "group") {
    emit("open-group", event.data);
    return;
  }
  emit("row-click", { data: event.data });
}

function handleRowDoubleClick(event: { data: LibraryBrowserEntry }) {
  if (event.data.kind === "group") {
    emit("open-group", event.data);
    return;
  }
  emit("row-dblclick", { data: event.data });
}

function handleRowContextMenu(event: { originalEvent: Event; data: LibraryBrowserEntry }) {
  if (event.data.kind === "group") {
    emit("group-contextmenu", { originalEvent: event.originalEvent, data: event.data });
    return;
  }
  emit("row-contextmenu", { originalEvent: event.originalEvent, data: event.data });
}

function handleSelectionUpdate(entry: LibraryBrowserEntry | null) {
  emit("update:selection", entry?.kind === "group" ? null : entry);
}

function handleContextSelectionUpdate(entry: LibraryBrowserEntry | null) {
  emit("update:tableContextSelection", entry?.kind === "group" ? null : entry);
}

function handleSort(event: { sortField?: string | ((item: LibraryBrowserEntry) => string); sortOrder?: number | null }) {
  if (typeof event.sortField !== "string" || !["name", "type", "status", "size", "updated_at"].includes(event.sortField)) {
    return;
  }
  emit("sort", {
    sortField: event.sortField as "name" | "type" | "status" | "size" | "updated_at",
    sortOrder: event.sortOrder === 1 ? 1 : -1,
  });
}

function handleFilter(event: DataTableFilterEvent) {
  const metadata = event.filters.status as DataTableOperatorFilterMetaData | undefined;
  const value = metadata?.constraints[0]?.value;
  emit("status-filter", typeof value === "string" ? value as LibraryIngestStatus : null);
}

function handleMobileStatusFilter(value: LibraryIngestStatus | null) {
  emit("status-filter", value);
}

watch(() => props.statusFilter, (status) => {
  filters.value.status.constraints[0]!.value = status;
});

function persistColumnLayout(state: { columnWidths?: unknown; tableWidth?: unknown }) {
  const layoutState: Record<string, string> = {};
  if (typeof state.columnWidths === "string") layoutState.columnWidths = state.columnWidths;
  if (typeof state.tableWidth === "string") layoutState.tableWidth = state.tableWidth;
  window.localStorage.setItem(tableStateKey.value, JSON.stringify(layoutState));
}

</script>

<template>
  <div class="library-pane flex h-full min-h-0 flex-col gap-2 bg-transparent px-2 py-2">
    <div class="flex min-h-0 flex-1 flex-col overflow-auto" @contextmenu.prevent="handleSurfaceContextMenu">
      <AsyncStateBlock
        :error="props.error"
        :loading="props.loading"
        :loading-title="$t('common.loading')"
        :loading-message="$t('library.loadingFiles')"
      >
        <template #error>
          <div class="grid justify-items-center gap-3 py-8 text-center">
            <AppStateMessage severity="error" :title="$t('common.error')">
              {{ props.error }}
            </AppStateMessage>
            <Button :class="toolPrimaryButtonClass" size="small" @click="emit('retry')">
              {{ $t("common.retry") }}
            </Button>
          </div>
        </template>

        <DataTable
          class="hidden min-h-0 flex-1 flex-col md:flex [&_.p-datatable-thead>tr>th]:!bg-(--p-content-background) [&_.p-paginator-content]:flex-nowrap [&_.p-paginator-rpp-dropdown]:!w-20 [&_.p-paginator-rpp-dropdown]:!min-w-20 [&_.p-paginator-rpp-dropdown]:!max-w-20 [&_.p-paginator-rpp-dropdown]:shrink-0"
          :selection="props.selection"
          :contextMenuSelection="props.tableContextSelection"
          :value="displayEntries"
          :first="props.first"
          :lazy="props.paginated"
          v-model:filters="filters"
          filter-display="menu"
          :paginator="props.paginated"
          :rows="props.pageSize"
          :rows-per-page-options="[25, 50, 100]"
          :sort-field="props.sortField"
          :sort-order="props.sortOrder"
          :total-records="props.totalRecords"
          data-key="key"
          selection-mode="single"
          context-menu
          resizable-columns
          column-resize-mode="expand"
          size="small"
          scrollable
          scroll-height="flex"
          state-storage="local"
          :state-key="tableStateKey"
          :table-class="props.compact ? 'w-full table-fixed' : 'min-w-[52rem]'"
          @update:selection="handleSelectionUpdate"
          @update:contextMenuSelection="handleContextSelectionUpdate"
          @row-click="handleRowClick"
          @row-dblclick="handleRowDoubleClick"
          @row-contextmenu="handleRowContextMenu"
          @page="emit('page', { first: $event.first, rows: $event.rows })"
          @sort="handleSort"
          @filter="handleFilter"
          @state-save="persistColumnLayout"
        >
          <template #empty>
            <div class="py-8 text-center text-sm text-(--p-text-muted-color)">
              {{ hasActiveResourceFilter ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
            </div>
          </template>

          <Column :header="$t('library.filename')" field="name" sort-field="name" :sortable="props.paginated" :class="props.compact ? 'w-[36%]' : 'min-w-72'">
            <template #body="{ data }">
              <div class="py-0 [padding-left:calc(var(--library-entry-depth,0)*0.9rem)]" :style="entryIndentStyle(data)">
                <div class="flex min-w-0 items-start gap-1.5">
                  <Button
                    v-if="data.kind === 'folder'"
                    class="library-folder-toggle mt-0.5 h-6 w-6 shrink-0 p-0"
                    type="button"
                    text
                    rounded
                    size="small"
                    severity="secondary"
                    :aria-label="isFolderExpanded(data) ? 'Collapse folder' : 'Expand folder'"
                    @click.stop="emit('toggle-folder', data)"
                  >
                    <span
                      class="transition-transform duration-150"
                      :class="{ 'rotate-90': isFolderExpanded(data) }"
                    >
                      &gt;
                    </span>
                  </Button>
                  <span v-else-if="data.kind === 'file'" class="mt-0.5 h-5 w-5 shrink-0" aria-hidden="true" />

                  <div class="grid min-w-0 flex-1 gap-1">
                    <span
                      class="block w-full cursor-pointer truncate text-left text-sm font-semibold leading-6 text-(--p-text-color)"
                      :data-entry-key="data.key"
                      @click.stop="openEntry(data)"
                    >
                      {{ data.name }}
                    </span>
                    <p v-if="data.kind === 'folder'" class="text-xs leading-4 text-(--p-text-muted-color)">
                      {{ $t("library.treeCounts", { folders: data.childFolderCount, files: data.fileCount }) }}
                    </p>
                    <p v-else-if="data.kind === 'group' && !props.hideGroupPaths" class="text-xs leading-4 text-(--p-text-muted-color)">
                      {{ data.path }}
                    </p>
                  </div>
                </div>
              </div>
            </template>
          </Column>

          <Column :header="$t('library.typeLabel')" sort-field="type" :sortable="props.paginated" :class="props.compact ? 'w-[11%]' : 'w-24'">
            <template #body="{ data }">
              <span class="text-sm text-(--p-text-muted-color)">{{ $t(resourceTypeLabel(data)) }}</span>
            </template>
          </Column>

          <Column
            :header="$t('library.statusLabel')"
            field="status"
            sort-field="status"
            :sortable="props.paginated"
            :show-filter-match-modes="false"
            :show-filter-operator="false"
            :show-add-button="false"
            :class="props.compact ? 'w-[16%]' : 'w-32'"
          >
            <template #filter="{ filterModel }">
              <Select
                v-model="filterModel.value"
                :aria-label="$t('library.filterByStatus')"
                class="w-full"
                :options="statusOptions"
                option-label="label"
                option-value="value"
              />
            </template>
            <template #body="{ data }">
              <Tag v-if="data.kind === 'group'" :value="data.visibility" severity="secondary" />
              <span v-else-if="data.kind === 'file'" class="inline-flex items-center gap-1.5">
                <span :class="statusTooltip(data) ? 'inline-flex cursor-help' : 'inline-flex'" :title="statusTooltip(data)">
                <Tag
                  :value="statusLabel(data.ingestStatus)"
                  :severity="statusSeverity(data.ingestStatus)"
                />
                </span>
                <Button
                  v-if="data.ingestStatus === 'failed' && !props.unavailableFileIds.includes(data.id)"
                  class="h-7 w-7 p-0"
                  icon="pi pi-refresh"
                  text
                  rounded
                  size="small"
                  severity="secondary"
                  :loading="isRetrying(data)"
                  :disabled="isRetrying(data)"
                  :aria-label="$t('common.retry')"
                  :title="$t('common.retry')"
                  @click.stop="emit('retry-entry', data)"
                />
              </span>
              <span v-else class="text-sm text-(--p-text-muted-color)">
                {{
                  data.processingCount > 0
                    ? $t(resourceStatusLabel(data), { count: data.processingCount })
                    : resourceStatusLabel(data)
                }}
              </span>
            </template>
          </Column>

          <Column :header="$t('library.sizeLabel')" sort-field="size" :sortable="props.paginated" :class="props.compact ? 'w-[12%]' : 'w-24'">
            <template #body="{ data }">
              <span class="tabular-nums text-sm text-(--p-text-muted-color)">{{ resourceSizeLabel(data) }}</span>
            </template>
          </Column>

          <Column :header="$t('library.updatedColumn')" sort-field="updated_at" :sortable="props.paginated" :class="props.compact ? 'w-[25%]' : 'w-36'">
            <template #body="{ data }">
              <span class="whitespace-nowrap text-sm text-(--p-text-muted-color)">
                {{ data.updatedAt ? formatTimestamp(data.updatedAt) : "—" }}
              </span>
            </template>
          </Column>

          <Column v-if="!props.hideActions" :header="$t('sources.table.action')" class="w-32">
            <template #body="{ data }">
              <div class="flex flex-nowrap items-center justify-start gap-1 whitespace-nowrap">
                <Button
                  v-if="data.kind === 'folder' && data.isSourceFolder"
                  text
                  size="small"
                  severity="secondary"
                  :aria-label="$t('sources.sync')"
                  :title="$t('sources.sync')"
                  @click.stop="emit('sync-source-folder', data)"
                >
                  {{ $t("sources.sync") }}
                </Button>
                <Button
                  text
                  size="small"
                  severity="secondary"
                  :aria-label="data.kind === 'group' ? $t('common.open') : data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  :title="data.kind === 'group' ? $t('common.open') : data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  @click.stop="openEntry(data)"
                >
                  {{ $t("common.open") }}
                </Button>
                <Button
                  v-if="data.kind === 'group'"
                  text
                  size="small"
                  severity="secondary"
                  :aria-label="$t('common.edit')"
                  :title="$t('common.edit')"
                  @click.stop="emit('edit-group', data)"
                >
                  {{ $t("common.edit") }}
                </Button>
                <Button
                  v-if="canMoveEntry(data)"
                  text
                  size="small"
                  severity="secondary"
                  :aria-label="$t('common.move')"
                  :title="$t('common.move')"
                  @click.stop="moveEntry(data)"
                >
                  {{ $t("common.move") }}
                </Button>
                <Button
                  v-if="canDeleteEntry(data)"
                  text
                  size="small"
                  severity="danger"
                  :aria-label="$t('common.delete')"
                  :title="$t('common.delete')"
                  @click.stop="deleteEntry(data)"
                >
                  {{ $t("common.delete") }}
                </Button>
              </div>
            </template>
          </Column>
        </DataTable>

        <div class="px-3 py-2 md:hidden">
          <label class="grid gap-1 text-sm font-medium text-(--p-text-muted-color)">
            <span>{{ $t("library.statusLabel") }}</span>
            <Select
              :model-value="props.statusFilter"
              :aria-label="$t('library.filterByStatus')"
              class="w-full"
              :options="statusOptions"
              option-label="label"
              option-value="value"
              @update:model-value="handleMobileStatusFilter"
            />
          </label>
        </div>

        <LibraryResourceCards
          :entries="displayEntries"
          :expanded-keys="props.expandedKeys"
          :hide-group-paths="props.hideGroupPaths"
          :resource-filter-active="hasActiveResourceFilter"
          :resource-search-query="props.resourceSearchQuery"
          :selection="props.selection"
          @delete="deleteEntry"
          @edit-group="emit('edit-group', $event)"
          @move="moveEntry"
          @open="openEntry"
          @row-click="handleRowClick({ data: $event })"
          @row-contextmenu="handleRowContextMenu"
          @row-dblclick="handleRowDoubleClick({ data: $event })"
          @toggle-folder="emit('toggle-folder', $event)"
        />

        <Paginator
          v-if="props.paginated"
          class="md:hidden"
          :first="props.first"
          :rows="props.pageSize"
          :rows-per-page-options="[25, 50, 100]"
          :total-records="props.totalRecords"
          @page="emit('page', { first: $event.first, rows: $event.rows })"
        />
      </AsyncStateBlock>
    </div>
  </div>
</template>
