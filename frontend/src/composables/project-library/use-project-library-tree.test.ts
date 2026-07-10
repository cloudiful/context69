import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryTreeResponse } from "../../services/api";
import { useProjectLibraryTree } from "./use-project-library-tree";

const mocks = vi.hoisted(() => ({
  getGroupLibraryTree: vi.fn(),
  showErrorToast: vi.fn(),
}));

vi.mock("../../services/api", () => ({
  apiClient: {
    getGroupLibraryTree: mocks.getGroupLibraryTree,
  },
}));

vi.mock("../use-error-toast", () => ({
  useErrorToast: () => mocks.showErrorToast,
}));

function libraryTree(groupPath: string): LibraryTreeResponse {
  return {
    root: {
      children: [],
      files: [],
      folder_id: null,
      group_key: groupPath.split("/").at(-1) ?? groupPath,
      group_path: groupPath,
      name: groupPath,
      parent_folder_id: null,
      path: groupPath,
      processing_count: 0,
      visibility: "private",
    },
  };
}

describe("useProjectLibraryTree", () => {
  beforeEach(() => {
    mocks.getGroupLibraryTree.mockReset();
    mocks.showErrorToast.mockReset();
  });

  it("resolves the current group path for every load", async () => {
    const groupPath = ref("stock/alpha");
    mocks.getGroupLibraryTree.mockImplementation(async (path: string) => libraryTree(path));
    const state = useProjectLibraryTree({
      groupPath,
      statusLabel: (status) => status,
      t: (key) => key,
    });

    await state.loadTree();
    groupPath.value = "stock/beta";
    state.resetTree();
    await state.loadTree();

    expect(mocks.getGroupLibraryTree).toHaveBeenNthCalledWith(1, "stock/alpha");
    expect(mocks.getGroupLibraryTree).toHaveBeenNthCalledWith(2, "stock/beta");
    expect(state.tree.value?.root.group_path).toBe("stock/beta");
  });

  it("keeps request failures distinct from an empty tree", async () => {
    const error = new Error("network failed");
    mocks.getGroupLibraryTree.mockRejectedValue(error);
    const state = useProjectLibraryTree({
      groupPath: "stock/alpha",
      statusLabel: (status) => status,
      t: (key) => key,
    });

    await state.loadTree();

    expect(state.tree.value).toBeNull();
    expect(state.treeError.value).toBe("library.loadFailed");
    expect(mocks.showErrorToast).toHaveBeenCalledWith(error, "library.loadFailed");
  });

  it("does not let a stale group request replace the current tree", async () => {
    const groupPath = ref("stock/alpha");
    let resolveAlpha: ((tree: LibraryTreeResponse) => void) | undefined;
    mocks.getGroupLibraryTree.mockImplementation((path: string) => {
      if (path === "stock/alpha") {
        return new Promise<LibraryTreeResponse>((resolve) => {
          resolveAlpha = resolve;
        });
      }
      return Promise.resolve(libraryTree(path));
    });
    const state = useProjectLibraryTree({
      groupPath,
      statusLabel: (status) => status,
      t: (key) => key,
    });

    const alphaLoad = state.loadTree();
    groupPath.value = "stock/beta";
    state.resetTree();
    await state.loadTree();
    resolveAlpha?.(libraryTree("stock/alpha"));
    await alphaLoad;

    expect(state.tree.value?.root.group_path).toBe("stock/beta");
  });
});
