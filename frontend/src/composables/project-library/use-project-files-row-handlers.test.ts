import { describe, expect, it, vi } from "vitest";

import type { LibraryFileSummary, LibraryFolderNode } from "../../services/api";
import type { ExplorerEntry, FileExplorerEntry, GroupExplorerEntry } from "../../types/library";
import { useProjectFilesRowHandlers } from "./use-project-files-row-handlers";

function fileEntry(overrides: Partial<FileExplorerEntry> = {}): FileExplorerEntry {
  return {
    key: "file:f1",
    kind: "file",
    id: "f1",
    depth: 0,
    name: "doc.txt",
    parentFolderId: null,
    path: "/",
    updatedAt: null,
    mediaType: "text/plain",
    sizeBytes: 10,
    ingestStatus: "succeeded",
    errorMessage: null,
    isSourceConfigFile: false,
    isSourceRecordFile: false,
    file: {} as LibraryFileSummary,
    ...overrides,
  };
}

function folderEntry(): Extract<ExplorerEntry, { kind: "folder" }> {
  return {
    key: "folder:fd1",
    kind: "folder",
    id: "fd1",
    depth: 0,
    name: "docs",
    parentFolderId: null,
    path: "/docs",
    updatedAt: null,
    childFolderCount: 0,
    fileCount: 1,
    isSourceFolder: false,
    isSourceRecordsFolder: false,
    processingCount: 0,
    folder: {} as LibraryFolderNode,
  };
}

function groupEntry(): GroupExplorerEntry {
  return {
    key: "group:7",
    kind: "group",
    id: 7,
    name: "stock",
    depth: 0,
    parentFolderId: null,
    path: "/stock",
    updatedAt: null,
    group: { group_id: 7 } as GroupExplorerEntry["group"],
    visibility: "private",
  };
}

function setup() {
  const tree = {
    resourceContextEntry: null as ExplorerEntry | null,
    selectedExplorerEntry: null as ExplorerEntry | null,
    selectFolder: vi.fn(async () => {}),
    toggleFolderExpansion: vi.fn(),
  };
  const actions = {
    releaseFileSource: vi.fn(),
    retryFile: vi.fn(async () => {}),
    revealPreviewForFile: vi.fn(async () => {}),
  };
  const sourceFolder = { openEditor: vi.fn(async () => {}) };
  const handlers = useProjectFilesRowHandlers({ tree, actions, sourceFolder });
  return { actions, handlers, sourceFolder, tree };
}

describe("useProjectFilesRowHandlers", () => {
  it("selects and reveals a file on row click", () => {
    const { actions, handlers, tree } = setup();
    const entry = fileEntry();

    handlers.handleExplorerRowClick({ data: entry });

    expect(tree.selectedExplorerEntry).toBe(entry);
    expect(actions.revealPreviewForFile).toHaveBeenCalledWith("f1");
    expect(tree.toggleFolderExpansion).not.toHaveBeenCalled();
  });

  it("selects and toggles a folder on row click without opening it", () => {
    const { actions, handlers, tree } = setup();
    const entry = folderEntry();

    handlers.handleExplorerRowClick({ data: entry });

    expect(tree.selectedExplorerEntry).toBe(entry);
    expect(tree.toggleFolderExpansion).toHaveBeenCalledWith("fd1");
    expect(tree.selectFolder).toHaveBeenCalledWith("fd1");
    expect(actions.revealPreviewForFile).not.toHaveBeenCalled();
  });

  it("opens a file on row double click without reselecting", () => {
    const { actions, handlers, tree } = setup();

    handlers.handleExplorerRowDoubleClick({ data: fileEntry() });

    expect(tree.selectedExplorerEntry).toBeNull();
    expect(actions.revealPreviewForFile).toHaveBeenCalledWith("f1");
  });

  it("opens the source editor for source config files", async () => {
    const { actions, handlers, sourceFolder, tree } = setup();
    const entry = fileEntry({ isSourceConfigFile: true });

    await handlers.openExplorerEntry(entry);

    expect(sourceFolder.openEditor).toHaveBeenCalledWith(entry);
    expect(actions.revealPreviewForFile).not.toHaveBeenCalled();
    expect(tree.toggleFolderExpansion).not.toHaveBeenCalled();
  });

  it("toggles and selects a folder when opening a folder entry", async () => {
    const { handlers, tree } = setup();
    const entry = folderEntry();

    await handlers.openExplorerEntry(entry);

    expect(tree.toggleFolderExpansion).toHaveBeenCalledWith("fd1");
    expect(tree.selectFolder).toHaveBeenCalledWith("fd1");
  });

  it("moves the resource context to the right-clicked row and clears the group context", () => {
    const { handlers, tree } = setup();
    const group = groupEntry();
    handlers.groupContextEntry.value = group;
    const entry = fileEntry();

    handlers.handleExplorerRowContextMenu({ originalEvent: new Event("contextmenu"), data: entry });

    expect(tree.resourceContextEntry).toBe(entry);
    expect(handlers.groupContextEntry.value).toBeNull();
  });

  it("sets the group context on group right-click and clears the resource context", () => {
    const { handlers, tree } = setup();
    const group = groupEntry();
    tree.resourceContextEntry = fileEntry();

    handlers.handleGroupRowContextMenu({ originalEvent: new Event("contextmenu"), data: group });

    expect(handlers.groupContextEntry.value).toStrictEqual(group);
    expect(tree.resourceContextEntry).toBeNull();
  });

  it("clears both contexts on a surface right-click", () => {
    const { handlers, tree } = setup();
    handlers.groupContextEntry.value = groupEntry();
    tree.resourceContextEntry = fileEntry();

    handlers.handleSurfaceContextMenu({ originalEvent: new MouseEvent("contextmenu") });

    expect(handlers.groupContextEntry.value).toBeNull();
    expect(tree.resourceContextEntry).toBeNull();
  });

  it("retries and releases only file entries", () => {
    const { actions, handlers } = setup();
    const file = fileEntry();
    const folder = folderEntry();

    handlers.retryExplorerEntry(file);
    handlers.retryExplorerEntry(folder);
    handlers.handleReleaseSource(file);
    handlers.handleReleaseSource(folder);

    expect(actions.retryFile).toHaveBeenCalledTimes(1);
    expect(actions.retryFile).toHaveBeenCalledWith("f1");
    expect(actions.releaseFileSource).toHaveBeenCalledTimes(1);
    expect(actions.releaseFileSource).toHaveBeenCalledWith("f1", "doc.txt");
  });
});
