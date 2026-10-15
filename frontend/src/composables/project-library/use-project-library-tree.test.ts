import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type LibraryTreeResponse } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { useProjectLibraryTree } from "./use-project-library-tree";

const getGroupLibraryTree = vi.spyOn(apiClient, "getGroupLibraryTree");

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
    getGroupLibraryTree.mockReset();
  });

  it("resolves the current group path for every load", async () => {
    const groupPath = ref("stock/alpha");
    getGroupLibraryTree.mockImplementation(async (path: string) => libraryTree(path));
    let state!: ReturnType<typeof useProjectLibraryTree>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryTree({ groupPath, statusLabel: (status) => status, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.loadTree();
    groupPath.value = "stock/beta";
    state.resetTree();
    await state.loadTree();

    expect(getGroupLibraryTree).toHaveBeenNthCalledWith(1, "stock/alpha");
    expect(getGroupLibraryTree).toHaveBeenNthCalledWith(2, "stock/beta");
    expect(state.tree.value?.root.group_path).toBe("stock/beta");
    wrapper.unmount();
  });

  it("keeps request failures distinct from an empty tree", async () => {
    const error = new Error("network failed");
    getGroupLibraryTree.mockRejectedValue(error);
    let state!: ReturnType<typeof useProjectLibraryTree>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryTree({ groupPath: "stock/alpha", statusLabel: (status) => status, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.loadTree();

    expect(state.tree.value).toBeNull();
    expect(state.treeError.value).toBe("library.loadFailed");
    wrapper.unmount();
  });

  it("does not let a stale group request replace the current tree", async () => {
    const groupPath = ref("stock/alpha");
    let resolveAlpha: ((tree: LibraryTreeResponse) => void) | undefined;
    getGroupLibraryTree.mockImplementation((path: string) => {
      if (path === "stock/alpha") {
        return new Promise<LibraryTreeResponse>((resolve) => {
          resolveAlpha = resolve;
        });
      }
      return Promise.resolve(libraryTree(path));
    });
    let state!: ReturnType<typeof useProjectLibraryTree>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryTree({ groupPath, statusLabel: (status) => status, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    const alphaLoad = state.loadTree();
    groupPath.value = "stock/beta";
    state.resetTree();
    await state.loadTree();
    resolveAlpha?.(libraryTree("stock/alpha"));
    await alphaLoad;

    expect(state.tree.value?.root.group_path).toBe("stock/beta");
    wrapper.unmount();
  });

  it("delegates user folder intent to the navigation callback when installed", async () => {
    const onNavigate = vi.fn(async () => {});
    let state!: ReturnType<typeof useProjectLibraryTree>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryTree({
          groupPath: "stock/alpha",
          statusLabel: (status) => status,
          t: (key) => key,
          onNavigate,
        });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.selectFolder("folder-1");

    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith("folder-1");
    expect(state.selectedFolderId.value).toBeNull();
    wrapper.unmount();
  });

  it("keeps direct selection when no navigation callback is installed", async () => {
    getGroupLibraryTree.mockImplementation(async (path: string) => libraryTree(path));
    let state!: ReturnType<typeof useProjectLibraryTree>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryTree({ groupPath: "stock/alpha", statusLabel: (status) => status, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.loadTree();
    await state.selectFolder("folder-1");

    expect(state.selectedFolderId.value).toBe("folder-1");
    wrapper.unmount();
  });

  it("keeps replaceSelection as the route-application primitive with a callback installed", async () => {
    const onNavigate = vi.fn(async () => {});
    let state!: ReturnType<typeof useProjectLibraryTree>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryTree({
          groupPath: "stock/alpha",
          statusLabel: (status) => status,
          t: (key) => key,
          onNavigate,
        });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.replaceSelection("folder-2", "file-9");

    expect(onNavigate).not.toHaveBeenCalled();
    expect(state.selectedFolderId.value).toBe("folder-2");
    expect(state.selectedFileId.value).toBe("file-9");
    wrapper.unmount();
  });
});
