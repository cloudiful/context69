import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";

import type { ContextMenuItem } from "@nuxt/ui";

import type { GroupResponse, LibraryFileSummary, LibraryFolderNode } from "../../services/api";
import type { ExplorerEntry, GroupExplorerEntry } from "../../types/library";
import { useProjectFilesContextMenu } from "./use-project-files-context-menu";

function fileEntry(): ExplorerEntry {
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
    group: { group_id: 7 } as GroupResponse,
    visibility: "private",
  };
}

function selectItem(items: ContextMenuItem[], label: string): ContextMenuItem {
  const item = items.find((candidate) => candidate.label === label);
  expect(item).toBeTruthy();
  return item!;
}

function setup() {
  const groupContextEntry = ref<GroupExplorerEntry | null>(null);
  const resourceContextEntry = ref<ExplorerEntry | null>(null);
  const actions = {
    unavailableFileIds: [] as string[],
    retryingFileIds: [] as string[],
    releasingFileIds: [] as string[],
    openCreateFolderDialog: vi.fn(),
    openCreateTextDialog: vi.fn(),
    openMoveFolderDialog: vi.fn(),
    openMoveFileDialog: vi.fn(),
    deleteFolder: vi.fn(),
    deleteFile: vi.fn(),
    retryFile: vi.fn(),
  };
  const sourceFolder = { sync: vi.fn(), openCreate: vi.fn() };
  const options = {
    t: (key: string) => key,
    groupContextEntry,
    resourceContextEntry: () => resourceContextEntry.value,
    actions,
    sourceFolder,
    selectFolder: vi.fn(),
    open: vi.fn(),
    releaseSource: vi.fn(),
    refresh: vi.fn(),
    upload: vi.fn(),
    emit: vi.fn(),
  };
  const menu = useProjectFilesContextMenu(options);
  return { actions, groupContextEntry, menu, options, resourceContextEntry, sourceFolder };
}

describe("useProjectFilesContextMenu", () => {
  it("falls back to the surface menu when no context entry is set", () => {
    const { menu } = setup();

    expect(menu.activeContextMenuItems.value[0].map((item) => item.label))
      .toEqual(["common.create", "common.upload", "sources.refresh"]);
  });

  it("prefers the resource menu, then the group menu, then the surface menu", () => {
    const { groupContextEntry, menu, resourceContextEntry } = setup();

    groupContextEntry.value = groupEntry();
    expect(selectItem(menu.activeContextMenuItems.value[0], "common.open")).toBeTruthy();

    resourceContextEntry.value = fileEntry();
    expect(selectItem(menu.activeContextMenuItems.value[0], "library.preview")).toBeTruthy();

    resourceContextEntry.value = null;
    groupContextEntry.value = null;
    expect(selectItem(menu.activeContextMenuItems.value[0], "common.create")).toBeTruthy();
  });

  it("maps group menu actions onto the panel emit events", () => {
    const { groupContextEntry, menu, options } = setup();
    const group = groupEntry();
    groupContextEntry.value = group;

    selectItem(menu.activeContextMenuItems.value[0], "common.open").onSelect?.(new Event("click"));
    expect(options.emit).toHaveBeenCalledWith("open-child-group", group.group);

    selectItem(menu.activeContextMenuItems.value[0], "common.delete").onSelect?.(new Event("click"));
    expect(options.emit).toHaveBeenCalledWith("delete-child-group", group.group);
  });

  it("drives surface create and upload entries from the shared callbacks", () => {
    const { menu, options } = setup();
    const items = menu.activeContextMenuItems.value[0];

    const children = selectItem(items, "common.create").children as ContextMenuItem[];
    selectItem(children, "groups.createChild").onSelect?.(new Event("click"));
    expect(options.emit).toHaveBeenCalledWith("create-child-group");

    selectItem(items, "common.upload").onSelect?.(new Event("click"));
    expect(options.upload).toHaveBeenCalledOnce();

    selectItem(items, "sources.refresh").onSelect?.(new Event("click"));
    expect(options.refresh).toHaveBeenCalledOnce();
  });

  it("reuses the tree, action and source-folder state for resource entries", () => {
    const { actions, menu, options, resourceContextEntry, sourceFolder } = setup();
    const folder = folderEntry();
    folder.isSourceFolder = true;
    resourceContextEntry.value = folder;

    const items = menu.activeContextMenuItems.value[0];
    selectItem(items, "library.openFolder").onSelect?.(new Event("click"));
    expect(options.selectFolder).toHaveBeenCalledWith("fd1");

    selectItem(items, "library.newFolder").onSelect?.(new Event("click"));
    expect(actions.openCreateFolderDialog).toHaveBeenCalledWith(folder.folder);

    selectItem(items, "sources.sync").onSelect?.(new Event("click"));
    expect(sourceFolder.sync).toHaveBeenCalledWith("fd1");
  });

  it("derives the create menu from the action and source-folder state", () => {
    const { actions, menu, sourceFolder } = setup();

    menu.createMenuItems.value[0].onSelect?.(new Event("click"));
    expect(actions.openCreateFolderDialog).toHaveBeenCalledOnce();

    menu.createMenuItems.value[2].onSelect?.(new Event("click"));
    expect(sourceFolder.openCreate).toHaveBeenCalledOnce();
  });
});
