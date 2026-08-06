import { computed } from "vue";

import type { LibraryIngestStatus } from "../../services/api";
import type { ExplorerEntry, GroupExplorerEntry, LibraryBrowserEntry } from "../../types/library";
import { formatBytes } from "../../utils/format";

interface ResourceTableState {
  entries: ExplorerEntry[];
  groupEntries: GroupExplorerEntry[];
  expandedKeys: Record<string, boolean>;
  resourceSearchQuery: string;
  retryingFileIds: string[];
  statusFilter: LibraryIngestStatus | null;
}

export function useLibraryResourceTable(
  props: ResourceTableState,
  t: (key: string) => string,
  statusLabel: (status: LibraryIngestStatus) => string,
) {
  const displayEntries = computed<LibraryBrowserEntry[]>(() => [...props.groupEntries, ...props.entries]);
  const hasActiveResourceFilter = computed(() => !!props.resourceSearchQuery || !!props.statusFilter);
  const statusOptions = computed(() => [
    { label: t("library.allStatuses"), value: null },
    ...(["pending", "running", "succeeded", "failed", "cancelled"] as const).map((value) => ({ label: statusLabel(value), value })),
  ]);

  return {
    displayEntries,
    hasActiveResourceFilter,
    statusOptions,
    resourceTypeLabel(entry: LibraryBrowserEntry) {
      if (entry.kind === "group") return t("groups.groupType");
      return t(entry.kind === "folder" ? "library.folderType" : "library.fileType");
    },
    resourceStatusLabel(entry: LibraryBrowserEntry) {
      if (entry.kind === "group") return entry.visibility;
      if (entry.kind === "folder") return entry.processingCount > 0 ? t("library.folderProcessing") : "-";
      return statusLabel(entry.ingestStatus);
    },
    resourceSizeLabel(entry: LibraryBrowserEntry) {
      return entry.kind === "file" ? formatBytes(entry.sizeBytes) : "-";
    },
    statusTooltip(entry: LibraryBrowserEntry) {
      return entry.kind === "file" && entry.ingestStatus === "failed" ? entry.errorMessage || undefined : undefined;
    },
    isRetrying(entry: LibraryBrowserEntry) {
      return entry.kind === "file" && props.retryingFileIds.includes(entry.id);
    },
    entryIndentStyle(entry: LibraryBrowserEntry) {
      return { "--library-entry-depth": String(entry.depth) };
    },
    isFolderExpanded(entry: LibraryBrowserEntry) {
      return entry.kind === "folder" && !!props.expandedKeys[entry.id ?? "__root__"];
    },
    canMoveEntry: canMutateEntry,
    canDeleteEntry: canMutateEntry,
  };
}

function canMutateEntry(entry: LibraryBrowserEntry) {
  if (entry.kind === "group") return true;
  if (entry.kind === "folder") return !entry.isSourceRecordsFolder;
  return !entry.isSourceConfigFile && !entry.isSourceRecordFile;
}
