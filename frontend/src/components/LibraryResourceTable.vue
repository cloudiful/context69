<script setup lang="ts">
import { computed } from "vue";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "primevue/tag";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import AppStateMessage from "./AppStateMessage.vue";
import LibraryResourceCards from "./LibraryResourceCards.vue";
import type { ExplorerEntry, GroupExplorerEntry, LibraryBrowserEntry } from "../types/library";
import { libraryRowActionButtonClass, libraryRowDangerActionButtonClass, toolPrimaryButtonClass } from "../ui/button-classes";
import { createLibraryStatusHelpers } from "../utils/library-status";
import { formatBytes, formatTimestamp } from "../utils/format";

const props = withDefaults(defineProps<{
  createFolderBusy: boolean;
  createSourceFolderBusy?: boolean;
  compact?: boolean;
  entries: ExplorerEntry[];
  error?: string | null;
  groupEntries?: GroupExplorerEntry[];
  hideActions?: boolean;
  hideGroupPaths?: boolean;
  expandedKeys: Record<string, boolean>;
  loading: boolean;
  resourceSearchQuery: string;
  selectedFolderReady: boolean;
  selection: ExplorerEntry | null;
  tableContextSelection: ExplorerEntry | null;
  uploadBusy: boolean;
}>(), {
  groupEntries: () => [],
  hideActions: false,
  hideGroupPaths: false,
  compact: false,
});

const emit = defineEmits<{
  "create-folder": [];
  "create-source-folder": [];
  "delete-group": [GroupExplorerEntry];
  "edit-group": [GroupExplorerEntry];
  "group-contextmenu": [{ originalEvent: Event; data: GroupExplorerEntry }];
  "move-group": [GroupExplorerEntry];
  "open-group": [GroupExplorerEntry];
  "open-entry": [ExplorerEntry];
  refresh: [];
  retry: [];
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

const { statusLabel, statusSeverity } = createLibraryStatusHelpers();
const displayEntries = computed<LibraryBrowserEntry[]>(() => [...props.groupEntries, ...props.entries]);

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

  if (target.closest("tr") || target.closest(".tool-card")) {
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

</script>

<template>
  <div class="library-pane library-pane-compact flex h-full flex-col">
    <div class="min-h-0 flex-1 overflow-auto" @contextmenu.prevent="handleSurfaceContextMenu">
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
          class="tool-table-desktop"
          :selection="props.selection"
          :contextMenuSelection="props.tableContextSelection"
          :value="displayEntries"
          data-key="key"
          selection-mode="single"
          context-menu
          resizable-columns
          column-resize-mode="expand"
          size="small"
          scrollable
          scroll-height="flex"
          state-storage="local"
          :state-key="props.compact ? 'context69:table:group-library:v3' : 'context69:table:library:v2'"
          :table-style="props.compact ? 'width: 100%; table-layout: fixed' : 'min-width: 52rem'"
          @update:selection="handleSelectionUpdate"
          @update:contextMenuSelection="handleContextSelectionUpdate"
          @row-click="handleRowClick"
          @row-dblclick="handleRowDoubleClick"
          @row-contextmenu="handleRowContextMenu"
        >
          <template #empty>
            <div class="py-8 text-center text-sm text-app-text-dim">
              {{ props.resourceSearchQuery ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
            </div>
          </template>

          <Column :header="$t('library.filename')" field="name" :style="props.compact ? 'width: 36%' : 'min-width: 18rem'">
            <template #body="{ data }">
              <div class="library-resource-record" :style="entryIndentStyle(data)">
                <div class="flex min-w-0 items-start gap-1.5">
                  <button
                    v-if="data.kind === 'folder'"
                    class="library-folder-toggle"
                    type="button"
                    :aria-label="isFolderExpanded(data) ? 'Collapse folder' : 'Expand folder'"
                    @click.stop="emit('toggle-folder', data)"
                  >
                    <span
                      class="library-folder-toggle-icon"
                      :class="{ 'library-folder-toggle-icon-expanded': isFolderExpanded(data) }"
                    >
                      &gt;
                    </span>
                  </button>
                  <span v-else-if="data.kind === 'file'" class="library-folder-toggle library-folder-toggle-placeholder" aria-hidden="true" />

                  <div class="grid min-w-0 gap-1">
                    <button
                      class="library-entry-button"
                      type="button"
                      :data-entry-key="data.key"
                      @click.stop="openEntry(data)"
                    >
                      {{ data.name }}
                    </button>
                    <p v-if="data.kind === 'folder'" class="text-xs leading-4 text-app-text-dim">
                      {{ $t("library.treeCounts", { folders: data.childFolderCount, files: data.fileCount }) }}
                    </p>
                    <p v-else-if="data.kind === 'group' && !props.hideGroupPaths" class="text-xs leading-4 text-app-text-dim">
                      {{ data.path }}
                    </p>
                  </div>
                </div>
              </div>
            </template>
          </Column>

          <Column :header="$t('library.typeLabel')" :style="props.compact ? 'width: 11%' : undefined" class="w-24">
            <template #body="{ data }">
              <span class="text-sm text-app-text-muted">{{ $t(resourceTypeLabel(data)) }}</span>
            </template>
          </Column>

          <Column :header="$t('library.statusLabel')" :style="props.compact ? 'width: 16%' : undefined" class="w-32">
            <template #body="{ data }">
              <Tag v-if="data.kind === 'group'" :value="data.visibility" severity="secondary" />
              <span v-else-if="data.kind === 'file'" :class="statusTooltip(data) ? 'inline-flex cursor-help' : 'inline-flex'" :title="statusTooltip(data)">
                <Tag
                  :value="statusLabel(data.ingestStatus)"
                  :severity="statusSeverity(data.ingestStatus)"
                />
              </span>
              <span v-else class="text-sm text-app-text-muted">
                {{
                  data.processingCount > 0
                    ? $t(resourceStatusLabel(data), { count: data.processingCount })
                    : resourceStatusLabel(data)
                }}
              </span>
            </template>
          </Column>

          <Column :header="$t('library.sizeLabel')" :style="props.compact ? 'width: 12%' : undefined" class="w-24">
            <template #body="{ data }">
              <span class="tabular-nums text-sm text-app-text-muted">{{ resourceSizeLabel(data) }}</span>
            </template>
          </Column>

          <Column :header="$t('library.updatedColumn')" :style="props.compact ? 'width: 25%' : undefined" class="w-36">
            <template #body="{ data }">
              <span class="whitespace-nowrap text-sm text-app-text-muted">
                {{ data.updatedAt ? formatTimestamp(data.updatedAt) : "—" }}
              </span>
            </template>
          </Column>

          <Column v-if="!props.hideActions" :header="$t('sources.table.action')" class="w-32">
            <template #body="{ data }">
              <div class="flex flex-nowrap items-center justify-start gap-1 whitespace-nowrap">
                <Button
                  v-if="data.kind === 'folder' && data.isSourceFolder"
                  unstyled
                  :class="libraryRowActionButtonClass"
                  :aria-label="$t('sources.sync')"
                  :title="$t('sources.sync')"
                  @click.stop="emit('sync-source-folder', data)"
                >
                  {{ $t("sources.sync") }}
                </Button>
                <Button
                  unstyled
                  :class="libraryRowActionButtonClass"
                  :aria-label="data.kind === 'group' ? $t('common.open') : data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  :title="data.kind === 'group' ? $t('common.open') : data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  @click.stop="openEntry(data)"
                >
                  {{ $t("common.open") }}
                </Button>
                <Button
                  v-if="data.kind === 'group'"
                  unstyled
                  :class="libraryRowActionButtonClass"
                  :aria-label="$t('common.edit')"
                  :title="$t('common.edit')"
                  @click.stop="emit('edit-group', data)"
                >
                  {{ $t("common.edit") }}
                </Button>
                <Button
                  v-if="canMoveEntry(data)"
                  unstyled
                  :class="libraryRowActionButtonClass"
                  :aria-label="$t('common.move')"
                  :title="$t('common.move')"
                  @click.stop="moveEntry(data)"
                >
                  {{ $t("common.move") }}
                </Button>
                <Button
                  v-if="canDeleteEntry(data)"
                  unstyled
                  :class="libraryRowDangerActionButtonClass"
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

        <LibraryResourceCards
          :entries="displayEntries"
          :expanded-keys="props.expandedKeys"
          :hide-group-paths="props.hideGroupPaths"
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
      </AsyncStateBlock>
    </div>
  </div>
</template>
