import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type LibraryFolderNode, type LibraryResourcePageResponse } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { useProjectLibraryPage } from "./use-project-library-page";

const getGroupLibraryResources = vi.spyOn(apiClient, "getGroupLibraryResources");

const root: LibraryFolderNode = {
  children: [],
  files: [],
  folder_id: null,
  group_key: "alpha",
  group_path: "stock/alpha",
  name: "alpha",
  parent_folder_id: null,
  path: "/",
  processing_count: 0,
  visibility: "private",
};

function response(): LibraryResourcePageResponse {
  return {
    items: [{
      child_folder_count: 0,
      created_at: "2026-07-11T10:00:00Z",
      file_count: 0,
      group_key: "alpha",
      group_path: "stock/alpha",
      id: "10000000-0000-0000-0000-000000000001",
      ingest_status: "succeeded",
      is_source_folder: false,
      is_source_records_folder: false,
      kind: "file",
      media_type: "text/plain",
      name: "latest.txt",
      parent_folder_id: null,
      processing_count: 0,
      size_bytes: 2048,
      updated_at: "2026-07-11T12:00:00Z",
      visibility: "private",
    }],
    pagination: { page: 2, page_size: 25, total: 80, total_pages: 4 },
  };
}

describe("useProjectLibraryPage", () => {
  beforeEach(() => {
    getGroupLibraryResources.mockReset();
  });

  it("loads a real backend page and forwards sorting parameters", async () => {
    getGroupLibraryResources.mockResolvedValue(response());
    const folder = ref<LibraryFolderNode | null>(root);
    let state!: ReturnType<typeof useProjectLibraryPage>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryPage({ groupPath: "stock/alpha", folder, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    state.query.value = "latest";
    await state.changeStatusFilter("failed");
    await state.changePage(25, 25);
    await state.changeSort("size", -1);

    expect(getGroupLibraryResources).toHaveBeenLastCalledWith("stock/alpha", {
      folderId: null,
      page: 1,
      pageSize: 25,
      query: "latest",
      status: "failed",
      sortBy: "size",
      sortDirection: "desc",
    });
    expect(state.entries.value[0]).toMatchObject({
      kind: "file",
      name: "latest.txt",
      sizeBytes: 2048,
    });
    expect(state.total.value).toBe(80);
    wrapper.unmount();
  });

  it("returns to the first page when the status filter changes", async () => {
    getGroupLibraryResources.mockResolvedValue(response());
    let state!: ReturnType<typeof useProjectLibraryPage>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProjectLibraryPage({ groupPath: "stock/alpha", folder: root, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    await state.changePage(50, 25);
    await state.changeStatusFilter("running");

    expect(state.page.value).toBe(1);
    expect(getGroupLibraryResources).toHaveBeenLastCalledWith("stock/alpha", expect.objectContaining({
      page: 1,
      status: "running",
    }));
    wrapper.unmount();
  });
});
