import type { ContextMenuItem } from "@nuxt/ui";

import type { ExplorerEntry, GroupExplorerEntry } from "../types/library";

type Translate = (key: string) => string;

export function resourceContextItems(options: {
  entry: ExplorerEntry | null;
  t: Translate;
  unavailableFileIds: string[];
  retryingFileIds: string[];
  open: (entry: ExplorerEntry) => void;
  selectFolder: (id: string | null) => void;
  createFolder: (entry: ExplorerEntry) => void;
  syncFolder: (id: string | null) => void;
  move: (entry: ExplorerEntry) => void;
  remove: (entry: ExplorerEntry) => void;
  refresh: (entry: ExplorerEntry) => void;
  retry: (id: string) => void;
}): ContextMenuItem[] {
  const { entry, t } = options;
  if (!entry) return [];
  if (entry.kind === "folder") {
    const items: ContextMenuItem[] = [
      { label: t("sources.refresh"), icon: "i-lucide-refresh-cw", onSelect: () => options.refresh(entry) },
      { label: t("library.openFolder"), icon: "i-lucide-folder-open", onSelect: () => options.selectFolder(entry.id) },
      { label: t("library.newFolder"), icon: "i-lucide-folder-plus", onSelect: () => options.createFolder(entry) },
    ];
    if (entry.isSourceFolder) items.push({ label: t("sources.sync"), icon: "i-lucide-refresh-cw", onSelect: () => options.syncFolder(entry.id) });
    if (!entry.isSourceRecordsFolder) {
      items.push({ label: t("common.move"), icon: "i-lucide-folder-input", onSelect: () => options.move(entry) });
      items.push({ label: t("common.delete"), icon: "i-lucide-trash-2", color: "error", onSelect: () => options.remove(entry) });
    }
    return items;
  }
  const items: ContextMenuItem[] = [
    { label: t("sources.refresh"), icon: "i-lucide-refresh-cw", onSelect: () => options.refresh(entry) },
    {
      label: entry.isSourceConfigFile ? t("library.editSourceConfig") : t("library.preview"),
      icon: entry.isSourceConfigFile ? "i-lucide-file-pen" : "i-lucide-eye",
      onSelect: () => options.open(entry),
    },
  ];
  if (["failed", "cancelled"].includes(entry.ingestStatus) && !options.unavailableFileIds.includes(entry.id)) {
    items.push({ label: options.retryingFileIds.includes(entry.id) ? t("library.retrying") : t("common.retry"), icon: "i-lucide-refresh-cw", onSelect: () => options.retry(entry.id) });
  }
  if (!entry.isSourceConfigFile && !entry.isSourceRecordFile) {
    items.push({ label: t("common.move"), icon: "i-lucide-file-input", onSelect: () => options.move(entry) });
    items.push({ label: t("common.delete"), icon: "i-lucide-trash-2", color: "error", onSelect: () => options.remove(entry) });
  }
  return items;
}

export function groupContextItems(group: GroupExplorerEntry | null, t: Translate, emit: (action: "open" | "edit" | "move" | "delete", group: GroupExplorerEntry) => void): ContextMenuItem[] {
  if (!group) return [];
  return [
    { label: t("common.open"), icon: "i-lucide-folder-open", onSelect: () => emit("open", group) },
    { label: t("common.edit"), icon: "i-lucide-pencil", onSelect: () => emit("edit", group) },
    { label: t("common.move"), icon: "i-lucide-folder-input", onSelect: () => emit("move", group) },
    { label: t("common.delete"), icon: "i-lucide-trash-2", color: "error", onSelect: () => emit("delete", group) },
  ];
}

export function surfaceContextItems(t: Translate, actions: {
  createGroup: () => void;
  createFolder: () => void;
  createText: () => void;
  createSource: () => void;
  upload: () => void;
  refresh: () => void;
}): ContextMenuItem[] {
  return [
    { label: t("common.create"), icon: "i-lucide-plus", children: [
      { label: t("groups.createChild"), icon: "i-lucide-network", onSelect: actions.createGroup },
      { label: t("library.newFolder"), icon: "i-lucide-folder-plus", onSelect: actions.createFolder },
      { label: t("library.newTextFile"), icon: "i-lucide-file-pen", onSelect: actions.createText },
      { label: t("library.newSourceFolder"), icon: "i-lucide-database", onSelect: actions.createSource },
    ] },
    { label: t("common.upload"), icon: "i-lucide-upload", onSelect: actions.upload },
    { label: t("sources.refresh"), icon: "i-lucide-refresh-cw", onSelect: actions.refresh },
  ];
}
