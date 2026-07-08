<script setup lang="ts">
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import FileUpload from "primevue/fileupload";
import Tag from "primevue/tag";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import type { ExplorerEntry } from "../types/library";
import { createLibraryStatusHelpers } from "../utils/library-status";
import { formatNumber, formatTimestamp } from "../utils/format";

const props = defineProps<{
  createFolderBusy: boolean;
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
  "open-entry": [ExplorerEntry];
  refresh: [];
  "row-click": [{ data: ExplorerEntry }];
  "row-contextmenu": [{ originalEvent: Event; data: ExplorerEntry }];
  "row-dblclick": [{ data: ExplorerEntry }];
  "toggle-folder": [ExplorerEntry];
  "move-entry": [ExplorerEntry];
  "delete-entry": [ExplorerEntry];
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

  return formatNumber(entry.sizeBytes);
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
</script>

<template>
  <div class="library-pane library-pane-compact flex h-full flex-col">
    <div class="library-browser-actions">
      <Button severity="secondary" variant="outlined" size="small" @click="emit('refresh')">
        {{ $t("sources.refresh") }}
      </Button>
      <Button
        id="library-open-create-folder"
        size="small"
        :disabled="createFolderBusy || !selectedFolderReady"
        @click="emit('create-folder')"
      >
        {{ createFolderBusy ? $t("library.creating") : $t("library.newFolder") }}
      </Button>
      <FileUpload
        mode="basic"
        name="library[]"
        custom-upload
        auto
        multiple
        :disabled="uploadBusy"
        accept=".pdf,.docx,.xlsx,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        :choose-label="uploadBusy ? $t('library.uploading') : $t('common.upload')"
        @select="emit('upload-select', $event)"
      />
    </div>

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

                  <span
                    class="library-resource-kind-icon"
                    :class="data.kind === 'folder' ? 'library-resource-kind-folder' : 'library-resource-kind-file'"
                    aria-hidden="true"
                  />

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
                    <p v-else class="library-resource-meta">{{ data.mediaType }}</p>
                    <p v-if="data.kind === 'file' && data.errorMessage" class="library-resource-error">
                      {{ data.errorMessage }}
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
              <Tag
                v-if="data.kind === 'file'"
                :value="statusLabel(data.ingestStatus)"
                :severity="statusSeverity(data.ingestStatus)"
              />
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
              <span class="text-sm text-app-text-muted">
                {{ data.updatedAt ? formatTimestamp(data.updatedAt) : "—" }}
              </span>
            </template>
          </Column>

          <Column :header="$t('sources.table.action')" class="w-44">
            <template #body="{ data }">
              <div class="library-row-actions">
                <Button
                  severity="secondary"
                  variant="text"
                  size="small"
                  :aria-label="data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  :title="data.kind === 'folder' ? $t('library.openFolder') : $t('library.preview')"
                  @click.stop="emit('open-entry', data)"
                >
                  {{ $t("common.open") }}
                </Button>
                <Button
                  severity="secondary"
                  variant="text"
                  size="small"
                  :aria-label="$t('common.move')"
                  :title="$t('common.move')"
                  @click.stop="emit('move-entry', data)"
                >
                  {{ $t("common.move") }}
                </Button>
                <Button
                  severity="danger"
                  variant="text"
                  size="small"
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
                <span
                  class="library-resource-kind-icon"
                  :class="entry.kind === 'folder' ? 'library-resource-kind-folder' : 'library-resource-kind-file'"
                  aria-hidden="true"
                />
                <div class="min-w-0">
                  <button
                    class="tool-card-title library-entry-button"
                    type="button"
                    :data-entry-key="entry.key"
                    @click.stop="emit('open-entry', entry)"
                  >
                    {{ entry.name }}
                  </button>
                  <p class="tool-card-subtitle">
                    {{ entry.kind === "folder" ? $t("library.treeCounts", { folders: entry.childFolderCount, files: entry.fileCount }) : entry.mediaType }}
                  </p>
                </div>
              </div>
              <Tag
                v-if="entry.kind === 'file'"
                class="tool-chip"
                :value="statusLabel(entry.ingestStatus)"
                :severity="statusSeverity(entry.ingestStatus)"
              />
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
            <p v-if="entry.kind === 'file' && entry.errorMessage" class="app-table-inline-error">
              {{ entry.errorMessage }}
            </p>
            <div class="tool-card-actions">
              <Button severity="secondary" variant="text" size="small" @click.stop="emit('open-entry', entry)">
                {{ $t("common.open") }}
              </Button>
              <Button severity="secondary" variant="text" size="small" @click.stop="emit('move-entry', entry)">
                {{ $t("common.move") }}
              </Button>
              <Button severity="danger" variant="text" size="small" @click.stop="emit('delete-entry', entry)">
                {{ $t("common.delete") }}
              </Button>
            </div>
          </article>
        </div>
      </AsyncStateBlock>
    </div>
  </div>
</template>
