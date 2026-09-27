import { ref } from "vue";

import type { ExplorerEntry, FileExplorerEntry, GroupExplorerEntry } from "../../types/library";

interface RowHandlerTree {
  resourceContextEntry: ExplorerEntry | null;
  selectedExplorerEntry: ExplorerEntry | null;
  selectFolder: (folderId: string | null) => Promise<void>;
  toggleFolderExpansion: (folderId: string | null) => void;
}

interface RowHandlerActions {
  releaseFileSource: (fileId: string, filename: string) => void;
  retryFile: (fileId: string) => Promise<void>;
  revealPreviewForFile: (fileId: string) => Promise<void>;
}

interface RowHandlerSourceFolder {
  openEditor: (entry: FileExplorerEntry) => Promise<void>;
}

interface Options {
  tree: RowHandlerTree;
  actions: RowHandlerActions;
  sourceFolder: RowHandlerSourceFolder;
}

export function useProjectFilesRowHandlers({ tree, actions, sourceFolder }: Options) {
  const groupContextEntry = ref<GroupExplorerEntry | null>(null);

  async function openExplorerEntry(entry: ExplorerEntry) {
    if (entry.kind === "folder") {
      tree.toggleFolderExpansion(entry.id);
      await tree.selectFolder(entry.id);
      return;
    }
    if (entry.isSourceConfigFile) {
      await sourceFolder.openEditor(entry);
      return;
    }
    await actions.revealPreviewForFile(entry.id);
  }

  function handleExplorerRowClick(event: { data: ExplorerEntry }) {
    const entry = event.data;
    tree.selectedExplorerEntry = entry;
    if (entry.kind === "folder") {
      tree.toggleFolderExpansion(entry.id);
      void tree.selectFolder(entry.id);
      return;
    }
    void openExplorerEntry(entry);
  }

  function handleExplorerRowDoubleClick(event: { data: ExplorerEntry }) {
    const entry = event.data;
    if (entry.kind === "folder") {
      tree.toggleFolderExpansion(entry.id);
      void tree.selectFolder(entry.id);
      return;
    }
    void openExplorerEntry(entry);
  }

  function handleExplorerRowContextMenu(event: { originalEvent: Event; data: ExplorerEntry }) {
    tree.resourceContextEntry = event.data;
    groupContextEntry.value = null;
  }

  function retryExplorerEntry(entry: ExplorerEntry) {
    if (entry.kind === "file") {
      void actions.retryFile(entry.id);
    }
  }

  function handleReleaseSource(entry: ExplorerEntry) {
    if (entry.kind === "file") {
      void actions.releaseFileSource(entry.id, entry.name);
    }
  }

  function handleGroupRowContextMenu(event: { originalEvent: Event; data: GroupExplorerEntry }) {
    groupContextEntry.value = event.data;
    tree.resourceContextEntry = null;
  }

  function handleSurfaceContextMenu(event: { originalEvent: MouseEvent }) {
    tree.resourceContextEntry = null;
    groupContextEntry.value = null;
  }

  return {
    groupContextEntry,
    handleExplorerRowClick,
    handleExplorerRowDoubleClick,
    handleExplorerRowContextMenu,
    handleGroupRowContextMenu,
    handleReleaseSource,
    handleSurfaceContextMenu,
    openExplorerEntry,
    retryExplorerEntry,
  };
}
