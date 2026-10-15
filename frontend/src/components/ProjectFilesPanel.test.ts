import { flushPromises, mount, shallowMount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import LibraryToolbar from "./LibraryToolbar.vue";
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

  async function mountPanel(router: ReturnType<typeof createRouter>, props: Record<string, unknown> = {}) {
    return shallowMount(ProjectFilesPanel, {
      attachTo: document.body,
      props: {
        childGroups: [],
        childGroupPage: { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } },
        childGroupSearch: "",
        groupPath: "stock",
        ...props,
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

  async function mountPanelWithToolbar(router: ReturnType<typeof createRouter>, props: Record<string, unknown> = {}) {
    return mount(ProjectFilesPanel, {
      attachTo: document.body,
      props: {
        childGroups: [],
        childGroupPage: { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } },
        childGroupSearch: "",
        groupPath: "stock",
        ...props,
      },
      global: {
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
        stubs: {
          UContextMenu: { template: "<div><slot /></div>" },
          ProjectFilesFileUpload: { template: "<div />" },
          ProjectFilesLibraryDialogs: { template: "<div />" },
          ProjectFilesPreviewModal: { template: "<div />" },
          ProjectFilesScopedSearchModal: { template: "<div />" },
          ProjectFilesRouteActions: { template: "<div />" },
          LibraryResourceTable: { template: "<div />" },
        },
      },
    });
  }

  function treeWithFolders() {
    return {
      root: {
        group_key: "stock",
        group_path: "stock",
        visibility: "private",
        folder_id: null,
        parent_folder_id: null,
        name: "Root",
        path: "/",
        processing_count: 0,
        children: [
          {
            group_key: "stock",
            group_path: "stock",
            visibility: "private",
            folder_id: "fd-1",
            parent_folder_id: null,
            name: "docs",
            path: "/docs",
            processing_count: 0,
            children: [
              {
                group_key: "stock",
                group_path: "stock",
                visibility: "private",
                folder_id: "fd-2",
                parent_folder_id: "fd-1",
                name: "nested",
                path: "/docs/nested",
                processing_count: 0,
                children: [],
                files: [],
              },
            ],
            files: [],
          },
        ],
        files: [],
      },
    };
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

  it("always renders the toolbar with the group breadcrumb and disabled Up at the root", async () => {
    mockLibraryApis();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock");
    await router.isReady();

    const wrapper = await mountPanelWithToolbar(router, { groupName: "Stock Group" });
    await flushPromises();

    const toolbar = wrapper.findComponent(LibraryToolbar);
    expect(toolbar.exists()).toBe(true);
    expect(toolbar.props("breadcrumbHome")).toMatchObject({ label: "Stock Group" });
    expect(toolbar.props("breadcrumbItems")).toEqual([]);
    expect(toolbar.props("canGoUp")).toBe(false);
    expect(wrapper.find('[data-testid="library-go-up"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("applies a direct folder deep link to the current page", async () => {
    vi.spyOn(apiClient, "getGroupLibraryTree").mockResolvedValue(treeWithFolders() as never);
    const getResources = vi.spyOn(apiClient, "getGroupLibraryResources").mockResolvedValue({
      items: [],
      pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 },
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock?folder=fd-1");
    await router.isReady();

    const wrapper = await mountPanel(router);
    await flushPromises();

    expect(router.currentRoute.value.query.folder).toBe("fd-1");
    expect(getResources).toHaveBeenCalledWith("stock", expect.objectContaining({ folderId: "fd-1" }));
    wrapper.unmount();
  });

  it("removes an invalid folder query and falls back to the root", async () => {
    mockLibraryApis();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock?folder=missing");
    await router.isReady();

    const wrapper = await mountPanel(router);
    await flushPromises();
    await flushPromises();

    expect(router.currentRoute.value.query.folder).toBeUndefined();
    wrapper.unmount();
  });

  it("uses the localized root name for scoped search at the virtual root", async () => {
    mockLibraryApis();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups/:groupPath", component: ProjectFilesPanel }],
    });
    await router.push("/groups/stock");
    await router.isReady();

    const wrapper = await mountPanelWithToolbar(router);
    await flushPromises();

    expect(wrapper.find('[data-testid="library-go-up"]').exists()).toBe(true);
    wrapper.unmount();
  });
});
