<script setup lang="ts">
import Tag from "./AppTag.vue";

import type { ExplorerEntry, GroupExplorerEntry, LibraryBrowserEntry } from "../types/library";
import { formatBytes, formatTimestamp } from "../utils/format";
import { createLibraryStatusHelpers } from "../utils/library-status";

const props = defineProps<{
  entries: LibraryBrowserEntry[];
  expandedKeys: Record<string, boolean>;
  hideGroupPaths?: boolean;
  resourceFilterActive?: boolean;
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
    <div v-if="entries.length === 0" class="px-3 py-8 text-center text-sm text-muted-color">
      {{ resourceFilterActive ? $t("library.noMatchingResources") : $t("library.emptyFolderMessage") }}
    </div>

    <article
      v-for="entry in entries"
      :key="entry.key"
      class="grid gap-[0.45rem] border-b border-surface bg-surface-0 dark:bg-surface-950 px-3 py-[0.65rem] text-sm last:border-b-0"
      data-testid="library-resource-card"
      data-library-card
      :class="{ 'bg-highlight': selection?.key === entry.key }"
      :style="entryIndentStyle(entry)"
      @click="emit('row-click', entry)"
      @dblclick="emit('row-dblclick', entry)"
      @contextmenu.prevent="emit('row-contextmenu', { originalEvent: $event, data: entry })"
    >
      <div class="flex min-w-0 items-start justify-between gap-3">
        <div class="flex min-w-0 items-start gap-1.5">
          <Button
            v-if="entry.kind === 'folder'"
            class="library-folder-toggle mt-0.5 h-6 w-6 shrink-0 p-0"
            type="button"
            text
            rounded
            size="small"
            severity="secondary"
            :aria-label="isFolderExpanded(entry) ? 'Collapse folder' : 'Expand folder'"
            @click.stop="emit('toggle-folder', entry)"
          >
            <span
              class="transition-transform duration-150"
              :class="{ 'rotate-90': isFolderExpanded(entry) }"
            >
              &gt;
            </span>
          </Button>
          <span v-else-if="entry.kind === 'file'" class="mt-0.5 h-5 w-5 shrink-0" aria-hidden="true" />
          <div class="min-w-0">
            <span
              class="block w-full cursor-pointer truncate text-left text-sm font-semibold leading-5 text-color"
              :data-entry-key="entry.key"
              @click.stop="emit('open', entry)"
            >
              {{ entry.name }}
            </span>
            <p v-if="entry.kind === 'folder'" class="mt-0.5 truncate text-xs leading-5 text-muted-color">
              {{ $t("library.treeCounts", { folders: entry.childFolderCount, files: entry.fileCount }) }}
            </p>
            <p v-else-if="entry.kind === 'group' && !props.hideGroupPaths" class="mt-0.5 truncate text-xs leading-5 text-muted-color">{{ entry.path }}</p>
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

      <dl v-if="entry.kind === 'file'" class="grid grid-cols-2 gap-x-3 gap-y-[0.35rem] text-xs text-muted-color">
        <div class="min-w-0">
          <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-muted-color">{{ $t("library.typeLabel") }}</dt>
          <dd class="mt-0.5 truncate text-muted-color">{{ $t("library.fileType") }}</dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-muted-color">{{ $t("library.sizeLabel") }}</dt>
          <dd class="mt-0.5 truncate text-muted-color">{{ formatBytes(entry.sizeBytes) }}</dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-muted-color">{{ $t("library.updatedColumn") }}</dt>
          <dd class="mt-0.5 truncate text-muted-color">{{ entry.updatedAt ? formatTimestamp(entry.updatedAt) : "—" }}</dd>
        </div>
      </dl>
      <div class="flex flex-wrap items-center justify-start gap-1">
        <Button text size="small" severity="secondary" @click.stop="emit('open', entry)">
          {{ $t("common.open") }}
        </Button>
        <Button
          v-if="entry.kind === 'group'"
          text
          size="small"
          severity="secondary"
          @click.stop="emit('edit-group', entry)"
        >
          {{ $t("common.edit") }}
        </Button>
        <Button text size="small" severity="secondary" @click.stop="emit('move', entry)">
          {{ $t("common.move") }}
        </Button>
        <Button text size="small" severity="danger" @click.stop="emit('delete', entry)">
          {{ $t("common.delete") }}
        </Button>
      </div>
    </article>
  </div>
</template>
