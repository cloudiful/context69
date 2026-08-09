import { flushPromises, mount, shallowMount } from "@vue/test-utils";
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

  function mockLibraryApis() {
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
      pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 },
    });
  }

  async function mountPanel(router: ReturnType<typeof createRouter>) {
    return shallowMount(ProjectFilesPanel, {
      attachTo: document.body,
      props: {
        childGroups: [],
        childGroupPage: { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } },
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
  }

  it("keeps the file browser in one layout root", async () => {
    mockLibraryApis();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock");
    await router.isReady();

    const wrapper = await mountPanel(router);

    await flushPromises();

    expect((wrapper.element as HTMLElement).classList.contains("project-files-panel")).toBe(true);
    wrapper.unmount();
  });

  it("opens the preview for a file deep link from the query", async () => {
    mockLibraryApis();
    const getFile = vi.spyOn(apiClient, "getGroupLibraryFile").mockResolvedValue({
      file_id: "file-1",
      filename: "doc.txt",
      media_type: "text/plain",
      size_bytes: 10,
      ingest_status: "succeeded",
      folder_id: null,
      folder_path: "/",
      group_key: "stock",
      group_path: "stock",
      sha256: "abc",
      visibility: "private",
      source_available: true,
      created_at: "2025-01-01T00:00:00Z",
      updated_at: "2025-01-01T00:00:00Z",
      sections: [],
      error_message: null,
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock?file=file-1");
    await router.isReady();

    const wrapper = await mountPanel(router);
    await flushPromises();

    expect(getFile).toHaveBeenCalledWith("stock", "file-1", expect.anything());
    wrapper.unmount();
  });

  it("keeps the deep link in the query after the initial reveal", async () => {
    mockLibraryApis();
    vi.spyOn(apiClient, "getGroupLibraryFile").mockResolvedValue({
      file_id: "file-1",
      filename: "doc.txt",
      media_type: "text/plain",
      size_bytes: 10,
      ingest_status: "succeeded",
      folder_id: null,
      folder_path: "/",
      group_key: "stock",
      group_path: "stock",
      sha256: "abc",
      visibility: "private",
      source_available: true,
      created_at: "2025-01-01T00:00:00Z",
      updated_at: "2025-01-01T00:00:00Z",
      sections: [],
      error_message: null,
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock?file=file-1");
    await router.isReady();

    const wrapper = await mountPanel(router);
    await flushPromises();

    expect(router.currentRoute.value.query.file).toBe("file-1");
    wrapper.unmount();
  });

  it("refreshes the tree and the current folder page from the table refresh event", async () => {
    mockLibraryApis();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock");
    await router.isReady();

    const wrapper = mount(ProjectFilesPanel, {
      attachTo: document.body,
      props: {
        childGroups: [],
        childGroupPage: { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } },
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
          LibraryToolbar: { template: "<div />" },
          LibraryResourceTable: {
            template: "<button class=\"table-refresh\" @click=\"$emit('refresh')\">refresh</button>",
          },
        },
      },
    });
    await flushPromises();

    const getTree = vi.spyOn(apiClient, "getGroupLibraryTree").mockClear();
    const getResources = vi.spyOn(apiClient, "getGroupLibraryResources").mockClear();

    await wrapper.get("button.table-refresh").trigger("click");
    await flushPromises();

    expect(getTree).toHaveBeenCalledTimes(1);
    expect(getResources).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });
});
