import { describe, expect, it } from "vitest";

import type { LibraryFolderNode } from "../../services/api";
import {
  buildLibraryFolderQuery,
  getParentFolderId,
  isValidLibraryFolder,
  normalizeLibraryFolderQuery,
  readLibraryFolderQuery,
  resolveLibraryFolderSelection,
} from "./use-project-library-navigation";

function folderNode(overrides: Partial<LibraryFolderNode> = {}): LibraryFolderNode {
  return {
    group_key: "stock",
    group_path: "stock",
    visibility: "private",
    folder_id: null,
    parent_folder_id: null,
    name: "Root",
    path: "/",
    processing_count: 0,
    children: [],
    files: [],
    ...overrides,
  };
}

describe("use-project-library-navigation", () => {
  it("normalizes folder query values with root tokens and arrays", () => {
    expect(normalizeLibraryFolderQuery(null)).toBeNull();
    expect(normalizeLibraryFolderQuery("")).toBeNull();
    expect(normalizeLibraryFolderQuery("  ")).toBeNull();
    expect(normalizeLibraryFolderQuery("null")).toBeNull();
    expect(normalizeLibraryFolderQuery("__root__")).toBeNull();
    expect(normalizeLibraryFolderQuery(["fd-1"])).toBe("fd-1");
    expect(normalizeLibraryFolderQuery(["", "fd-2"])).toBeNull();
    expect(normalizeLibraryFolderQuery("fd-1")).toBe("fd-1");
  });

  it("reads the folder query key only", () => {
    expect(readLibraryFolderQuery({ folder: "fd-1" })).toBe("fd-1");
    expect(readLibraryFolderQuery({})).toBeNull();
    expect(readLibraryFolderQuery({ folder: ["fd-2"] })).toBe("fd-2");
  });

  it("builds folder navigation queries with file cleared and root removed", () => {
    expect(buildLibraryFolderQuery({ from: "search", file: "f-1", other: "keep" }, "fd-9")).toEqual({
      from: "search",
      other: "keep",
      folder: "fd-9",
    });
    expect(buildLibraryFolderQuery({ from: "search", file: "f-1", folder: "old" }, null)).toEqual({
      from: "search",
    });
  });

  it("validates folder ids against the loaded tree", () => {
    const root = folderNode({
      children: [folderNode({ folder_id: "fd-1", parent_folder_id: null, name: "docs", path: "/docs" })],
    });
    expect(isValidLibraryFolder(root, null)).toBe(true);
    expect(isValidLibraryFolder(root, "fd-1")).toBe(true);
    expect(isValidLibraryFolder(root, "missing")).toBe(false);
    expect(isValidLibraryFolder(null, "fd-1")).toBe(false);
    expect(resolveLibraryFolderSelection(root, "missing")).toBeNull();
    expect(resolveLibraryFolderSelection(root, "fd-1")).toBe("fd-1");
  });

  it("resolves the parent folder from a trail", () => {
    const root = folderNode();
    const child = folderNode({ folder_id: "fd-1", parent_folder_id: null, name: "docs", path: "/docs" });
    expect(getParentFolderId([root])).toBeNull();
    expect(getParentFolderId([root, child])).toBeNull();
    const nested = folderNode({ folder_id: "fd-2", parent_folder_id: "fd-1", name: "nested", path: "/docs/nested" });
    expect(getParentFolderId([root, child, nested])).toBe("fd-1");
  });
});
