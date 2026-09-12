import { describe, expect, it, vi } from "vitest";

import type { LibraryFileSummary, LibraryFolderNode } from "../services/api";
import type { ExplorerEntry } from "../types/library";
import { resourceContextItems, surfaceContextItems } from "./project-files-context-menu";

function fileEntry(): Extract<ExplorerEntry, { kind: "file" }> {
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

function resourceOptions(
  entry: ExplorerEntry,
  overrides: Partial<Parameters<typeof resourceContextItems>[0]> = {},
): Parameters<typeof resourceContextItems>[0] {
  return {
    entry,
    t: (key) => key,
    unavailableFileIds: [],
    retryingFileIds: [],
    releasingFileIds: [],
    open: vi.fn(),
    selectFolder: vi.fn(),
    createFolder: vi.fn(),
    syncFolder: vi.fn(),
    move: vi.fn(),
    remove: vi.fn(),
    refresh: vi.fn(),
    retry: vi.fn(),
    releaseSource: vi.fn(),
    ...overrides,
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
      releasingFileIds: [],
      open: vi.fn(),
      selectFolder: vi.fn(),
      createFolder: vi.fn(),
      syncFolder: vi.fn(),
      move: vi.fn(),
      remove: vi.fn(),
      refresh,
      retry: vi.fn(),
      releaseSource: vi.fn(),
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
      releasingFileIds: [],
      open: vi.fn(),
      selectFolder: vi.fn(),
      createFolder: vi.fn(),
      syncFolder: vi.fn(),
      move: vi.fn(),
      remove: vi.fn(),
      refresh,
      retry: vi.fn(),
      releaseSource: vi.fn(),
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

  it("offers retry for failed and stale pending/running files", () => {
    for (const status of ["pending", "running", "failed", "cancelled"] as const) {
      const entry = fileEntry();
      entry.ingestStatus = status;
      const items = resourceContextItems(resourceOptions(entry));
      expect(items.some((item) => item.label === "common.retry"), status).toBe(true);
    }
  });

  it("does not offer retry for succeeded files", () => {
    const items = resourceContextItems(resourceOptions(fileEntry()));
    expect(items.some((item) => item.label === "common.retry")).toBe(false);
  });

  it("hides retry when the source is unavailable", () => {
    const entry = fileEntry();
    entry.ingestStatus = "failed";
    const items = resourceContextItems(resourceOptions(entry, { unavailableFileIds: [entry.id] }));
    expect(items.some((item) => item.label === "common.retry")).toBe(false);
  });

  it("offers source release for a succeeded file with an available source", () => {
    const entry = fileEntry();
    const releaseSource = vi.fn();
    const items = resourceContextItems(resourceOptions(entry, { releaseSource }));

    const releaseItem = items.find((item) => item.label === "library.releaseSource");
    expect(releaseItem).toBeTruthy();
    releaseItem!.onSelect?.(new Event("click"));
    expect(releaseSource).toHaveBeenCalledWith(entry);
  });

  it("hides source release when the source is unavailable or the file is a control file", () => {
    const unavailable = fileEntry();
    const unavailableItems = resourceContextItems(resourceOptions(unavailable, {
      unavailableFileIds: [unavailable.id],
    }));
    expect(unavailableItems.some((item) => item.label === "library.releaseSource")).toBe(false);

    const controlFile = fileEntry();
    controlFile.isSourceConfigFile = true;
    const controlItems = resourceContextItems(resourceOptions(controlFile));
    expect(controlItems.some((item) => item.label === "library.releaseSource")).toBe(false);
  });
});
