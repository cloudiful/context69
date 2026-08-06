import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type LibraryFolderNode } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { useProjectLibraryActions } from "./use-project-library-actions";

const getGroupLibraryFile = vi.spyOn(apiClient, "getGroupLibraryFile");
const submitTask = vi.spyOn(apiClient, "submitTask");

const root: LibraryFolderNode = {
  children: [],
  files: [],
  folder_id: null,
  group_key: "group",
  group_path: "group",
  name: "group",
  parent_folder_id: null,
  path: "/",
  processing_count: 0,
  visibility: "private",
};

describe("useProjectLibraryActions retry", () => {
  beforeEach(() => {
    getGroupLibraryFile.mockReset();
    getGroupLibraryFile.mockResolvedValue({ source_available: true } as never);
    submitTask.mockReset();
  });

  it("prevents duplicate retry requests and refreshes the tree once", async () => {
    submitTask.mockResolvedValue({ task_id: "task-id", item_ids: ["item-id"] } as never);
    const loadTree = vi.fn().mockResolvedValue(undefined);
    const groupPath = ref("group-a");
    let state!: ReturnType<typeof useProjectLibraryActions>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryActions({
          groupPath,
          loadTree,
          moveOptions: ref([]),
          replaceSelection: vi.fn().mockResolvedValue(undefined),
          selectFile: vi.fn().mockResolvedValue(undefined),
          selectedFolder: ref(root),
          selectedFileId: ref("file-id"),
          t: (key) => key,
          updateExpandedForFolder: vi.fn(),
          previewDocked: ref(false),
          previewDialogVisible: ref(false),
        });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    groupPath.value = "group-b";
    const first = state.retryFile("file-id");
    await state.retryFile("file-id");
    expect(submitTask).toHaveBeenCalledTimes(1);
    expect(submitTask).toHaveBeenCalledWith({
      kind: "retry_file_batch",
      group_path: "group-b",
      items: [{ file_id: "file-id" }],
    });
    expect(state.retryingFileIds.value).toEqual(["file-id"]);

    await first;

    expect(loadTree).toHaveBeenCalledOnce();
    expect(state.retryingFileIds.value).toEqual([]);
    wrapper.unmount();
  });

  it("does not retry when the stored original is missing", async () => {
    getGroupLibraryFile.mockResolvedValue({ source_available: false } as never);
    let state!: ReturnType<typeof useProjectLibraryActions>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryActions({
          groupPath: ref("group"),
          loadTree: vi.fn(),
          moveOptions: ref([]),
          replaceSelection: vi.fn(),
          selectFile: vi.fn(),
          selectedFolder: ref(root),
          selectedFileId: ref("file-id"),
          t: (key) => key,
          updateExpandedForFolder: vi.fn(),
          previewDocked: ref(false),
          previewDialogVisible: ref(false),
        });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.retryFile("file-id");

    expect(submitTask).not.toHaveBeenCalled();
    expect(state.unavailableFileIds.value).toEqual(["file-id"]);
    wrapper.unmount();
  });
});
