<script setup lang="ts">
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "primevue/tag";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import type { ExplorerEntry } from "../types/library";
import { libraryRowActionButtonClass, libraryRowDangerActionButtonClass } from "../ui/button-classes";
import { createLibraryStatusHelpers } from "../utils/library-status";
import { formatBytes, formatTimestamp } from "../utils/format";

const props = defineProps<{
  createFolderBusy: boolean;
  createSourceFolderBusy?: boolean;
  entries: ExplorerEntry[];
  expandedKeys: Record<string, boolean>;
  loading: boolean;
  resourceSearchQuery: string;
  selectedFolderReady: boolean;
  selection: ExplorerEntry | null;
  tableContextSelection: ExplorerEntry | null;
  uploadBusy: boolean;
}>();

const emit = defineEmits<{
  "create-folder": [];
  "create-source-folder": [];
  "open-entry": [ExplorerEntry];
  refresh: [];
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

function resourceTypeLabel(entry: ExplorerEntry): string {
  return entry.kind === "folder" ? "library.folderType" : "library.fileType";
}

function resourceStatusLabel(entry: ExplorerEntry): string {
  if (entry.kind === "folder") {
    return entry.processingCount > 0
      ? `library.folderProcessing`
      : "—";
  }

  return statusLabel(entry.ingestStatus);
}

function resourceSizeLabel(entry: ExplorerEntry): string {
  if (entry.kind === "folder") {
    return "—";
  }

  return formatBytes(entry.sizeBytes);
}

function statusTooltip(entry: ExplorerEntry): string | undefined {
  if (entry.kind !== "file" || entry.ingestStatus !== "failed") {
    return undefined;
  }

  return entry.errorMessage || undefined;
}

function entryIndentStyle(entry: ExplorerEntry) {
  return {
    "--library-entry-depth": String(entry.depth),
  };
}

function isFolderExpanded(entry: ExplorerEntry): boolean {
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

function canMoveEntry(entry: ExplorerEntry): boolean {
  if (entry.kind === "folder") {
    return !entry.isSourceRecordsFolder;
  }
  return !entry.isSourceConfigFile && !entry.isSourceRecordFile;
}

function canDeleteEntry(entry: ExplorerEntry): boolean {
  if (entry.kind === "folder") {
    return !entry.isSourceRecordsFolder;
  }
  return !entry.isSourceConfigFile && !entry.isSourceRecordFile;
}

</script>

<template>
  <div class="library-pane library-pane-compact flex h-full flex-col">
    <div class="split-panel-body flex-1 overflow-auto" @contextmenu.prevent="handleSurfaceContextMenu">
      <AsyncStateBlock
        :loading="props.loading"
        :loading-title="$t('common.loading')"
        :loading-message="$t('library.loadingFiles')"
      >
        <DataTable
          class="tool-table-desktop"
          :selection="props.selection"
          :contextMenuSelection="props.tableContextSelection"
          :value="props.entries"
          data-key="key"
          selection-mode="single"
          context-menu
          size="small"
          scrollable
          scroll-height="flex"
          table-style="min-width: 52rem"
          @update:selection="emit('update:selection', $event)"
          @update:contextMenuSelection="emit('update:tableContextSelection', $event)"
          @row-click="emit('row-click', $event)"
          @row-dblclick="emit('row-dblclick', $event)"
          @row-contextmenu="emit('row-contextmenu', $event)"
        >
          <template #empty>
            <div class="py-8 text-center text-sm text-app-text-dim">
              {{ props.resourceSearchQuery ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
            </div>
          </template>

          <Column :header="$t('library.filename')" field="name" style="min-width: 18rem">
            <template #body="{ data }">
              <div class="library-resource-record" :style="entryIndentStyle(data)">
                <div class="library-resource-main">
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
                  <span v-else class="library-folder-toggle library-folder-toggle-placeholder" aria-hidden="true" />

                  <div class="library-resource-copy">
                    <button
                      class="library-entry-button"
                      type="button"
                      :data-entry-key="data.key"
                      @click.stop="emit('open-entry', data)"
                    >
                      {{ data.name }}
                    </button>
                    <p v-if="data.kind === 'folder'" class="library-resource-meta">
                      {{ $t("library.treeCounts", { folders: data.childFolderCount, files: data.fileCount }) }}
                    </p>
                  </div>
                </div>
              </div>
            </template>
          </Column>

          <Column :header="$t('library.typeLabel')" class="w-24">
            <template #body="{ data }">
              <span class="text-sm text-app-text-muted">{{ $t(resourceTypeLabel(data)) }}</span>
            </template>
          </Column>

          <Column :header="$t('library.statusLabel')" class="w-32">
            <template #body="{ data }">
              <span v-if="data.kind === 'file'" :class="statusTooltip(data) ? 'inline-flex cursor-help' : 'inline-flex'" :title="statusTooltip(data)">
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

          <Column :header="$t('library.sizeLabel')" class="w-24">
            <template #body="{ data }">
              <span class="tabular-nums text-sm text-app-text-muted">{{ resourceSizeLabel(data) }}</span>
            </template>
          </Column>

          <Column :header="$t('library.updatedColumn')" class="w-36">
            <template #body="{ data }">
              <span class="whitespace-nowrap text-sm text-app-text-muted">
                {{ data.updatedAt ? formatTimestamp(data.updatedAt) : "—" }}
              </span>
            </template>
          </Column>

          <Column :header="$t('sources.table.action')" class="w-32">
            <template #body="{ data }">
              <div class="library-row-actions">
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
                  :aria-label="data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  :title="data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  @click.stop="emit('open-entry', data)"
                >
                  {{ $t("common.open") }}
                </Button>
                <Button
                  v-if="canMoveEntry(data)"
                  unstyled
                  :class="libraryRowActionButtonClass"
                  :aria-label="$t('common.move')"
                  :title="$t('common.move')"
                  @click.stop="emit('move-entry', data)"
                >
                  {{ $t("common.move") }}
                </Button>
                <Button
                  v-if="canDeleteEntry(data)"
                  unstyled
                  :class="libraryRowDangerActionButtonClass"
                  :aria-label="$t('common.delete')"
                  :title="$t('common.delete')"
                  @click.stop="emit('delete-entry', data)"
                >
                  {{ $t("common.delete") }}
                </Button>
              </div>
            </template>
          </Column>
        </DataTable>

        <div class="tool-card-list library-card-list">
          <div v-if="props.entries.length === 0" class="tool-empty">
            {{ props.resourceSearchQuery ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
          </div>

          <article
            v-for="entry in props.entries"
            :key="entry.key"
            class="tool-card"
            :class="{ 'tool-card-selected': props.selection?.key === entry.key }"
            :style="entryIndentStyle(entry)"
            @click="emit('row-click', { data: entry })"
            @dblclick="emit('row-dblclick', { data: entry })"
            @contextmenu.prevent="emit('row-contextmenu', { originalEvent: $event, data: entry })"
          >
            <div class="tool-card-header">
              <div class="library-resource-main">
                <button
                  v-if="entry.kind === 'folder'"
                  class="library-folder-toggle"
                  type="button"
                  :aria-label="isFolderExpanded(entry) ? 'Collapse folder' : 'Expand folder'"
                  @click.stop="emit('toggle-folder', entry)"
                >
                  <span
                    class="library-folder-toggle-icon"
                    :class="{ 'library-folder-toggle-icon-expanded': isFolderExpanded(entry) }"
                  >
                    &gt;
                  </span>
                </button>
                <span v-else class="library-folder-toggle library-folder-toggle-placeholder" aria-hidden="true" />
                <div class="min-w-0">
                  <button
                    class="tool-card-title library-entry-button"
                    type="button"
                    :data-entry-key="entry.key"
                    @click.stop="emit('open-entry', entry)"
                  >
                    {{ entry.name }}
                  </button>
                  <p v-if="entry.kind === 'folder'" class="tool-card-subtitle">
                    {{ $t("library.treeCounts", { folders: entry.childFolderCount, files: entry.fileCount }) }}
                  </p>
                </div>
              </div>
              <span
                v-if="entry.kind === 'file'"
                :class="statusTooltip(entry) ? 'inline-flex cursor-help' : 'inline-flex'"
                :title="statusTooltip(entry)"
              >
                <Tag
                  class="tool-chip"
                  :value="statusLabel(entry.ingestStatus)"
                  :severity="statusSeverity(entry.ingestStatus)"
                />
              </span>
            </div>

            <dl v-if="entry.kind === 'file'" class="tool-meta-grid">
              <div>
                <dt>{{ $t("library.typeLabel") }}</dt>
                <dd>{{ $t(resourceTypeLabel(entry)) }}</dd>
              </div>
              <div>
                <dt>{{ $t("library.sizeLabel") }}</dt>
                <dd>{{ resourceSizeLabel(entry) }}</dd>
              </div>
              <div>
                <dt>{{ $t("library.updatedColumn") }}</dt>
                <dd>{{ entry.updatedAt ? formatTimestamp(entry.updatedAt) : "—" }}</dd>
              </div>
            </dl>
            <div class="tool-card-actions">
              <Button unstyled :class="libraryRowActionButtonClass" @click.stop="emit('open-entry', entry)">
                {{ $t("common.open") }}
              </Button>
              <Button unstyled :class="libraryRowActionButtonClass" @click.stop="emit('move-entry', entry)">
                {{ $t("common.move") }}
              </Button>
              <Button unstyled :class="libraryRowDangerActionButtonClass" @click.stop="emit('delete-entry', entry)">
                {{ $t("common.delete") }}
              </Button>
            </div>
          </article>
        </div>
      </AsyncStateBlock>
    </div>
  </div>
</template>
