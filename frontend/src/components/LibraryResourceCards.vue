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
  <div class="hidden grid-cols-1 md:hidden library-card-list">
    <div v-if="entries.length === 0" class="px-3 py-8 text-center text-sm text-app-text-dim">
      {{ resourceSearchQuery ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
    </div>

    <article
      v-for="entry in entries"
      :key="entry.key"
      class="grid gap-[0.45rem] border-b border-app-border/70 bg-app-surface px-3 py-[0.65rem] text-sm last:border-b-0"
      data-testid="library-resource-card"
      data-library-card
      :class="{ 'bg-[color-mix(in_srgb,var(--color-app-surface-soft)_54%,var(--color-app-surface)_46%)]': selection?.key === entry.key }"
      :style="entryIndentStyle(entry)"
      @click="emit('row-click', entry)"
      @dblclick="emit('row-dblclick', entry)"
      @contextmenu.prevent="emit('row-contextmenu', { originalEvent: $event, data: entry })"
    >
      <div class="flex min-w-0 items-start justify-between gap-3">
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
              class="block w-full truncate text-left text-sm font-semibold leading-5 text-app-text transition hover:text-app-text-muted"
              type="button"
              :data-entry-key="entry.key"
              @click.stop="emit('open', entry)"
            >
              {{ entry.name }}
            </button>
            <p v-if="entry.kind === 'folder'" class="mt-0.5 truncate text-xs leading-5 text-app-text-dim">
              {{ $t("library.treeCounts", { folders: entry.childFolderCount, files: entry.fileCount }) }}
            </p>
            <p v-else-if="entry.kind === 'group' && !props.hideGroupPaths" class="mt-0.5 truncate text-xs leading-5 text-app-text-dim">{{ entry.path }}</p>
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

      <dl v-if="entry.kind === 'file'" class="grid grid-cols-2 gap-x-3 gap-y-[0.35rem] text-xs text-app-text-dim">
        <div class="min-w-0">
          <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ $t("library.typeLabel") }}</dt>
          <dd class="mt-0.5 truncate text-app-text-muted">{{ $t("library.fileType") }}</dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ $t("library.sizeLabel") }}</dt>
          <dd class="mt-0.5 truncate text-app-text-muted">{{ formatBytes(entry.sizeBytes) }}</dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ $t("library.updatedColumn") }}</dt>
          <dd class="mt-0.5 truncate text-app-text-muted">{{ entry.updatedAt ? formatTimestamp(entry.updatedAt) : "—" }}</dd>
        </div>
      </dl>
      <div class="flex flex-wrap items-center justify-start gap-1">
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
