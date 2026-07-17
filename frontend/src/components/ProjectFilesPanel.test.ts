import { flushPromises, shallowMount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import ProjectFilesPanel from "./ProjectFilesPanel.vue";

describe("ProjectFilesPanel", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = '<div id="app-route-actions"></div>';
  });

  it("keeps the file browser in one layout root", async () => {
    vi.spyOn(apiClient, "getGroupLibraryTree").mockResolvedValue({
      root: {
        group_key: "stock",
        group_path: "stock",
        visibility: "private",
        folder_id: null,
        parent_folder_id: null,
        name: "stock",
        path: "/",
        processing_count: 0,
        children: [],
        files: [],
      },
    });
    vi.spyOn(apiClient, "getGroupLibraryResources").mockResolvedValue({
      items: [],
      page: 1,
      page_size: 50,
      total: 0,
      total_pages: 0,
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock");
    await router.isReady();

    const wrapper = shallowMount(ProjectFilesPanel, {
      attachTo: document.body,
      props: {
        childGroups: [],
        childGroupPage: { items: [], page: 1, page_size: 50, total: 0, total_pages: 0 },
        childGroupSearch: "",
        groupPath: "stock",
      },
      global: {
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
        stubs: {
          UContextMenu: { template: "<div><slot /></div>" },
          UFileUpload: { template: "<input />" },
          UInput: { template: "<input />" },
          UModal: { template: "<div><slot name=\"body\" /></div>" },
        },
      },
    });

    await flushPromises();

    expect((wrapper.element as HTMLElement).classList.contains("project-files-panel")).toBe(true);
    wrapper.unmount();
  });
});
