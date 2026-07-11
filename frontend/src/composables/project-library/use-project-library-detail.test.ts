import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useProjectLibraryDetail } from "./use-project-library-detail";

const mocks = vi.hoisted(() => ({
  getGroupLibraryFile: vi.fn(),
  getGroupLibraryJob: vi.fn(),
  showErrorToast: vi.fn(),
}));

vi.mock("../../services/api", () => ({
  apiClient: {
    getGroupLibraryFile: mocks.getGroupLibraryFile,
    getGroupLibraryJob: mocks.getGroupLibraryJob,
  },
}));
vi.mock("../use-error-toast", () => ({ useErrorToast: () => mocks.showErrorToast }));

describe("useProjectLibraryDetail", () => {
  beforeEach(() => {
    mocks.getGroupLibraryFile.mockReset();
    mocks.getGroupLibraryJob.mockReset();
    mocks.showErrorToast.mockReset();
  });

  it("loads details with the current group path after navigation", async () => {
    mocks.getGroupLibraryFile.mockResolvedValue({ jobs: [], sections: [] });
    const groupPath = ref("stock/group-a");
    const wrapper = mount(defineComponent({
      setup() {
        return useProjectLibraryDetail({
          groupPath,
          loadTree: vi.fn().mockResolvedValue(undefined),
          selectedFileId: ref(null),
          t: (key) => key,
        });
      },
      template: "<div />",
    }));

    groupPath.value = "stock/group-b";
    await wrapper.vm.loadDetail("file-id");

    expect(mocks.getGroupLibraryFile).toHaveBeenCalledWith(
      "stock/group-b",
      "file-id",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });
});
