import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { useProjectLibraryDetail } from "./use-project-library-detail";

const getGroupLibraryFile = vi.spyOn(apiClient, "getGroupLibraryFile");

describe("useProjectLibraryDetail", () => {
  beforeEach(() => {
    getGroupLibraryFile.mockReset();
  });

  it("loads details with the current group path after navigation", async () => {
    getGroupLibraryFile.mockResolvedValue({ jobs: [], sections: [] } as never);
    const groupPath = ref("stock/group-a");
    const wrapper = mount(defineComponent({
      setup() {
        return useProjectLibraryDetail({
          groupPath,
          t: (key) => key,
        });
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    groupPath.value = "stock/group-b";
    await wrapper.vm.loadDetail("file-id");

    expect(getGroupLibraryFile).toHaveBeenCalledWith(
      "stock/group-b",
      "file-id",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });
});
