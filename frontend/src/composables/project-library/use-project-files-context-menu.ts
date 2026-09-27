import { computed, type EmitFn, type Ref } from "vue";
import type { ContextMenuItem } from "@nuxt/ui";

import { groupContextItems, resourceContextItems, surfaceContextItems } from "../../components/project-files-context-menu";
import type { GroupResponse, LibraryFileSummary, LibraryFolderNode } from "../../services/api";
import type { ExplorerEntry, GroupExplorerEntry } from "../../types/library";

type Translate = (key: string) => string;

interface ResourceActions {
  unavailableFileIds: string[];
  retryingFileIds: string[];
  releasingFileIds: string[];
  openCreateFolderDialog: (folder?: LibraryFolderNode | null) => void;
  openCreateTextDialog: (folder?: LibraryFolderNode | null) => void;
  openMoveFolderDialog: (folder: LibraryFolderNode) => void;
  openMoveFileDialog: (file: LibraryFileSummary) => void;
  deleteFolder: (folder: LibraryFolderNode) => void;
  deleteFile: (file: LibraryFileSummary) => void;
  retryFile: (fileId: string) => void;
}

interface SourceFolderActions {
  sync: (folderId: string | null) => void;
  openCreate: () => void;
}

interface GroupMenuEvents {
  "create-child-group": [];
  "open-child-group": [GroupResponse];
  "edit-child-group": [GroupResponse];
  "move-child-group": [GroupResponse];
  "delete-child-group": [GroupResponse];
}

interface Options {
  t: Translate;
  groupContextEntry: Ref<GroupExplorerEntry | null>;
  resourceContextEntry: () => ExplorerEntry | null;
  actions: ResourceActions;
  sourceFolder: SourceFolderActions;
  selectFolder: (folderId: string | null) => void;
  open: (entry: ExplorerEntry) => void;
  releaseSource: (entry: ExplorerEntry) => void;
  refresh: () => void;
  upload: () => void;
  emit: EmitFn<GroupMenuEvents>;
}

export function useProjectFilesContextMenu(options: Options) {
  const resourceMenuItems = computed(() => resourceContextItems({
    entry: options.resourceContextEntry(),
    t: options.t,
    unavailableFileIds: options.actions.unavailableFileIds,
    retryingFileIds: options.actions.retryingFileIds,
    releasingFileIds: options.actions.releasingFileIds,
    open: options.open,
    selectFolder: options.selectFolder,
    createFolder: (entry) => entry.kind === "folder" && options.actions.openCreateFolderDialog(entry.folder),
    syncFolder: (id) => { void options.sourceFolder.sync(id); },
    move: (entry) => entry.kind === "folder" ? options.actions.openMoveFolderDialog(entry.folder) : entry.kind === "file" && options.actions.openMoveFileDialog(entry.file),
    remove: (entry) => entry.kind === "folder" ? void options.actions.deleteFolder(entry.folder) : entry.kind === "file" && void options.actions.deleteFile(entry.file),
    refresh: options.refresh,
    retry: (id) => { void options.actions.retryFile(id); },
    releaseSource: options.releaseSource,
  }));

  const groupMenuItems = computed(() => groupContextItems(options.groupContextEntry.value, options.t, (action, entry) => {
    if (action === "open") options.emit("open-child-group", entry.group);
    else if (action === "edit") options.emit("edit-child-group", entry.group);
    else if (action === "move") options.emit("move-child-group", entry.group);
    else options.emit("delete-child-group", entry.group);
  }));

  const createMenuItems = computed<ContextMenuItem[]>(() => [
    { label: options.t("library.newFolder"), icon: "i-lucide-folder-plus", onSelect: () => options.actions.openCreateFolderDialog() },
    { label: options.t("library.newTextFile"), icon: "i-lucide-file-plus", onSelect: () => options.actions.openCreateTextDialog() },
    { label: options.t("library.newSourceFolder"), icon: "i-lucide-database-plus", onSelect: () => options.sourceFolder.openCreate() },
  ]);

  const surfaceMenuItems = computed(() => surfaceContextItems(options.t, {
    createGroup: () => options.emit("create-child-group"),
    createFolder: () => options.actions.openCreateFolderDialog(),
    createText: () => options.actions.openCreateTextDialog(),
    createSource: () => options.sourceFolder.openCreate(),
    upload: options.upload,
    refresh: options.refresh,
  }));

  const activeContextMenuItems = computed<ContextMenuItem[][]>(() => [
    (options.resourceContextEntry()
      ? resourceMenuItems.value
      : options.groupContextEntry.value ? groupMenuItems.value : surfaceMenuItems.value) as ContextMenuItem[],
  ]);

  return {
    activeContextMenuItems,
    createMenuItems,
  };
}
