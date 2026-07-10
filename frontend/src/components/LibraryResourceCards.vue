<script setup lang="ts">
import Tag from "primevue/tag";

import type { ExplorerEntry, GroupExplorerEntry, LibraryBrowserEntry } from "../types/library";
import { libraryRowActionButtonClass, libraryRowDangerActionButtonClass } from "../ui/button-classes";
import { formatBytes, formatTimestamp } from "../utils/format";
import { createLibraryStatusHelpers } from "../utils/library-status";

const props = defineProps<{
  entries: LibraryBrowserEntry[];
  expandedKeys: Record<string, boolean>;
  hideGroupPaths?: boolean;
  resourceSearchQuery: string;
  selection: ExplorerEntry | null;
}>();

const emit = defineEmits<{
  delete: [LibraryBrowserEntry];
  "edit-group": [GroupExplorerEntry];
  move: [LibraryBrowserEntry];
  open: [LibraryBrowserEntry];
  "row-click": [LibraryBrowserEntry];
  "row-contextmenu": [{ originalEvent: Event; data: LibraryBrowserEntry }];
  "row-dblclick": [LibraryBrowserEntry];
  "toggle-folder": [ExplorerEntry];
}>();

const { statusLabel, statusSeverity } = createLibraryStatusHelpers();

function entryIndentStyle(entry: LibraryBrowserEntry) {
  return { "--library-entry-depth": String(entry.depth) };
}

function isFolderExpanded(entry: LibraryBrowserEntry) {
  return entry.kind === "folder" && !!props.expandedKeys[entry.id ?? "__root__"];
}
</script>

<template>
  <div class="tool-card-list library-card-list">
    <div v-if="entries.length === 0" class="tool-empty">
      {{ resourceSearchQuery ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
    </div>

    <article
      v-for="entry in entries"
      :key="entry.key"
      class="tool-card"
      :class="{ 'tool-card-selected': selection?.key === entry.key }"
      :style="entryIndentStyle(entry)"
      @click="emit('row-click', entry)"
      @dblclick="emit('row-dblclick', entry)"
      @contextmenu.prevent="emit('row-contextmenu', { originalEvent: $event, data: entry })"
    >
      <div class="tool-card-header">
        <div class="flex min-w-0 items-start gap-1.5">
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
          <span v-else-if="entry.kind === 'file'" class="library-folder-toggle library-folder-toggle-placeholder" aria-hidden="true" />
          <div class="min-w-0">
            <button
              class="tool-card-title library-entry-button"
              type="button"
              :data-entry-key="entry.key"
              @click.stop="emit('open', entry)"
            >
              {{ entry.name }}
            </button>
            <p v-if="entry.kind === 'folder'" class="tool-card-subtitle">
              {{ $t("library.treeCounts", { folders: entry.childFolderCount, files: entry.fileCount }) }}
            </p>
            <p v-else-if="entry.kind === 'group' && !props.hideGroupPaths" class="tool-card-subtitle">{{ entry.path }}</p>
          </div>
        </div>
        <Tag v-if="entry.kind === 'group'" class="tool-chip" :value="entry.visibility" severity="secondary" />
        <span
          v-else-if="entry.kind === 'file'"
          :class="entry.errorMessage && entry.ingestStatus === 'failed' ? 'inline-flex cursor-help' : 'inline-flex'"
          :title="entry.ingestStatus === 'failed' ? entry.errorMessage || undefined : undefined"
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
          <dd>{{ $t("library.fileType") }}</dd>
        </div>
        <div>
          <dt>{{ $t("library.sizeLabel") }}</dt>
          <dd>{{ formatBytes(entry.sizeBytes) }}</dd>
        </div>
        <div>
          <dt>{{ $t("library.updatedColumn") }}</dt>
          <dd>{{ entry.updatedAt ? formatTimestamp(entry.updatedAt) : "—" }}</dd>
        </div>
      </dl>
      <div class="tool-card-actions">
        <Button unstyled :class="libraryRowActionButtonClass" @click.stop="emit('open', entry)">
          {{ $t("common.open") }}
        </Button>
        <Button
          v-if="entry.kind === 'group'"
          unstyled
          :class="libraryRowActionButtonClass"
          @click.stop="emit('edit-group', entry)"
        >
          {{ $t("common.edit") }}
        </Button>
        <Button unstyled :class="libraryRowActionButtonClass" @click.stop="emit('move', entry)">
          {{ $t("common.move") }}
        </Button>
        <Button unstyled :class="libraryRowDangerActionButtonClass" @click.stop="emit('delete', entry)">
          {{ $t("common.delete") }}
        </Button>
      </div>
    </article>
  </div>
</template>
