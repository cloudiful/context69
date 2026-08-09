import { describe, expect, it, vi } from "vitest";

import type { LibraryFileSummary, LibraryFolderNode } from "../services/api";
import type { ExplorerEntry } from "../types/library";
import { resourceContextItems, surfaceContextItems } from "./project-files-context-menu";

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

function folderEntry(): ExplorerEntry {
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

describe("project files context menu", () => {
  it("offers a refresh action on file entries", () => {
    const refresh = vi.fn();
    const entry = fileEntry();
    const items = resourceContextItems({
      entry,
      t: (key) => key,
      unavailableFileIds: [],
      retryingFileIds: [],
      open: vi.fn(),
      selectFolder: vi.fn(),
      createFolder: vi.fn(),
      syncFolder: vi.fn(),
      move: vi.fn(),
      remove: vi.fn(),
      refresh,
      retry: vi.fn(),
    });

    const refreshItem = items.find((item) => item.label === "sources.refresh");
    expect(refreshItem).toBeTruthy();
    refreshItem!.onSelect?.(new Event("click"));
    expect(refresh).toHaveBeenCalledWith(entry);
  });

  it("offers a refresh action on folder entries", () => {
    const refresh = vi.fn();
    const entry = folderEntry();
    const items = resourceContextItems({
      entry,
      t: (key) => key,
      unavailableFileIds: [],
      retryingFileIds: [],
      open: vi.fn(),
      selectFolder: vi.fn(),
      createFolder: vi.fn(),
      syncFolder: vi.fn(),
      move: vi.fn(),
      remove: vi.fn(),
      refresh,
      retry: vi.fn(),
    });

    const refreshItem = items.find((item) => item.label === "sources.refresh");
    expect(refreshItem).toBeTruthy();
    refreshItem!.onSelect?.(new Event("click"));
    expect(refresh).toHaveBeenCalledWith(entry);
  });

  it("keeps a refresh action on the empty surface", () => {
    const refresh = vi.fn();
    const items = surfaceContextItems((key) => key, {
      createGroup: vi.fn(),
      createFolder: vi.fn(),
      createText: vi.fn(),
      createSource: vi.fn(),
      upload: vi.fn(),
      refresh,
    });

    const refreshItem = items.find((item) => item.label === "sources.refresh");
    expect(refreshItem).toBeTruthy();
    refreshItem!.onSelect?.(new Event("click"));
    expect(refresh).toHaveBeenCalledOnce();
  });
});
