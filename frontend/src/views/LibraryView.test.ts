import { mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { apiClient } from "../services/api";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import type { LibraryResourceItem, LibraryResourcePageResponse } from "../services/api";
import type { ExplorerEntry } from "../types/library";
import LibraryView from "./LibraryView.vue";

interface TestLibraryMenuItem {
  onSelect: () => void;
  label: string;
}

const treeResponse = {
  root: {
    folder_id: null,
    parent_folder_id: null,
    name: "Root",
    path: "/",
    processing_count: 0,
    files: [],
    children: [
      {
        folder_id: "folder-1",
        parent_folder_id: null,
        name: "Policies",
        path: "/Policies",
        processing_count: 0,
        files: [],
        children: [],
      },
    ],
  },
};

const rootResource: LibraryResourceItem = {
  child_folder_count: 0,
  created_at: "2026-04-07T00:00:00Z",
  file_count: 1,
  group_key: "library",
  group_path: "library",
  id: "folder-1",
  is_source_folder: false,
  is_source_records_folder: false,
  kind: "folder",
  name: "Policies",
  parent_folder_id: null,
  processing_count: 0,
  updated_at: "2026-04-07T00:00:00Z",
  visibility: "private",
};

const fileResource: LibraryResourceItem = {
  child_folder_count: 0,
  created_at: "2026-04-07T00:00:00Z",
  file_count: 0,
  group_key: "library",
  group_path: "library",
  id: "file-1",
  ingest_status: "succeeded",
  is_source_folder: false,
  is_source_records_folder: false,
  kind: "file",
  media_type: "application/pdf",
  name: "Quarterly Report.pdf",
  parent_folder_id: "folder-1",
  processing_count: 0,
  size_bytes: 4096,
  updated_at: "2026-04-07T00:00:00Z",
  visibility: "private",
};

function resourcePage(items: LibraryResourceItem[]): LibraryResourcePageResponse {
  return {
    items,
    page: 1,
    page_size: 50,
    total: items.length,
    total_pages: items.length > 0 ? 1 : 0,
  };
}

function mockLibraryResources() {
  return vi.spyOn(apiClient, "getLibraryResources").mockImplementation(async (params) => (
    resourcePage(params.folderId === "folder-1" ? [fileResource] : [rootResource])
  ));
}

describe("LibraryView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("loads the tree and opens file details from the route query", async () => {
    const getLibraryTree = vi.spyOn(apiClient, "getLibraryTree").mockResolvedValue(treeResponse as never);
    const getLibraryResources = mockLibraryResources();
    const getLibraryFile = vi.spyOn(apiClient, "getLibraryFile").mockResolvedValue({
      file_id: "file-1",
      folder_id: "folder-1",
      folder_path: "/Policies",
      filename: "Quarterly Report.pdf",
      media_type: "application/pdf",
      size_bytes: 4096,
      sha256: "abc",
      ingest_status: "succeeded",
      error_message: null,
      created_at: "2026-04-07T00:00:00Z",
      updated_at: "2026-04-07T00:00:00Z",
      ingested_at: "2026-04-07T00:00:00Z",
      jobs: [],
      sections: [
        {
          content_format: "plain_text",
          document_id: 11,
          section_key: "document",
          section_label: "Quarterly Report.pdf",
          sort_order: 0,
          title: "Quarterly Report.pdf",
          preview_text: "Parsed report text",
        },
      ],
    } as never);

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/library", name: "library", component: LibraryView }],
    });

    router.push("/library?file=file-1");
    await router.isReady();

    const wrapper = mount(LibraryView, {
      attachTo: document.body,
      global: {
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
      },
    });
    await vi.waitFor(() => {
      expect(getLibraryTree).toHaveBeenCalled();
      expect(getLibraryResources).toHaveBeenCalled();
      expect(getLibraryFile).toHaveBeenCalledWith("file-1", expect.any(Object));
      expect(router.currentRoute.value.query.folder).toBe("folder-1");
    });

    expect(wrapper.find(".library-topbar").exists()).toBe(false);
    expect(wrapper.find(".library-toolbar-shell").exists()).toBe(true);
    expect(wrapper.text()).not.toContain("Contents");
    expect(wrapper.text()).toContain("Policies");
    expect(wrapper.text()).toContain("Quarterly Report.pdf");
    expect(document.body.textContent).toContain("/Policies");
    expect(document.body.textContent).toContain("Parsed report text");
    wrapper.unmount();
  });

  it("selects a folder and creates a subfolder from the browser actions", async () => {
    const getLibraryTree = vi.spyOn(apiClient, "getLibraryTree");
    const getLibraryResources = mockLibraryResources();
    getLibraryTree
      .mockResolvedValueOnce(treeResponse as never)
      .mockResolvedValueOnce({
        root: {
          ...treeResponse.root,
          children: [
            {
              ...treeResponse.root.children[0],
              children: [
                {
                  folder_id: "folder-2",
                  parent_folder_id: "folder-1",
                  name: "Archives",
                  path: "/Policies/Archives",
                  processing_count: 0,
                  files: [],
                  children: [],
                },
              ],
            },
          ],
        },
      } as never);
    const createLibraryFolder = vi.spyOn(apiClient, "createLibraryFolder").mockResolvedValue({
      folder_id: "folder-2",
      parent_folder_id: "folder-1",
      name: "Archives",
      path: "/Policies/Archives",
      created_at: "2026-04-07T00:00:00Z",
      updated_at: "2026-04-07T00:00:00Z",
    } as never);
    vi.spyOn(apiClient, "getLibraryFile").mockResolvedValue(null as never);

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/library", name: "library", component: LibraryView }],
    });

    router.push("/library");
    await router.isReady();

    const wrapper = mount(LibraryView, {
      attachTo: document.body,
      global: {
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
      },
    });
    await vi.waitFor(() => {
      expect(getLibraryTree).toHaveBeenCalled();
      expect(getLibraryResources).toHaveBeenCalled();
      expect(wrapper.find("[data-entry-key='folder:folder-1']").exists()).toBe(true);
    });

    expect(wrapper.find(".library-folder-toggle").exists()).toBe(true);
    expect(wrapper.text()).toContain("Policies");
    await wrapper.get("[data-entry-key='folder:folder-1']").trigger("click");
    await vi.waitFor(() => {
      expect(router.currentRoute.value.query.folder).toBe("folder-1");
    });

    expect(wrapper.text()).toContain("Policies");

    await wrapper.get("#library-open-create-folder").trigger("click");
    await vi.waitFor(() => {
      expect(document.body.querySelector("#library-create-folder-name")).not.toBeNull();
    });

    const dialogInput = document.body.querySelector("#library-create-folder-name") as HTMLInputElement | null;
    expect(dialogInput).not.toBeNull();
    dialogInput!.value = "Archives";
    dialogInput!.dispatchEvent(new Event("input", { bubbles: true }));
    await wrapper.vm.$nextTick();

    const dialogButtons = [...document.body.querySelectorAll("button")];
    const dialogCreateButton = dialogButtons.find((button) => button.textContent?.includes("Create Folder"));
    expect(dialogCreateButton).toBeTruthy();
    dialogCreateButton!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await vi.waitFor(() => {
      expect(createLibraryFolder).toHaveBeenCalledWith({
        parent_folder_id: "folder-1",
        name: "Archives",
      });
      expect(router.currentRoute.value.query.folder).toBe("folder-2");
    });

    expect(wrapper.text()).toContain("Archives");
    wrapper.unmount();
  });

  it("opens the preview dialog when preview is triggered from the file context menu", async () => {
    vi.spyOn(apiClient, "getLibraryTree").mockResolvedValue(treeResponse as never);
    mockLibraryResources();
    const getLibraryFile = vi.spyOn(apiClient, "getLibraryFile").mockResolvedValue({
      file_id: "file-1",
      folder_id: "folder-1",
      folder_path: "/Policies",
      filename: "Quarterly Report.pdf",
      media_type: "application/pdf",
      size_bytes: 4096,
      sha256: "abc",
      ingest_status: "succeeded",
      error_message: null,
      created_at: "2026-04-07T00:00:00Z",
      updated_at: "2026-04-07T00:00:00Z",
      ingested_at: "2026-04-07T00:00:00Z",
      jobs: [],
      sections: [
        {
          content_format: "plain_text",
          document_id: 11,
          section_key: "document",
          section_label: "Quarterly Report.pdf",
          sort_order: 0,
          title: "Quarterly Report.pdf",
          preview_text: "Parsed report text",
        },
      ],
    } as never);

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/library", name: "library", component: LibraryView }],
    });

    router.push("/library?folder=folder-1");
    await router.isReady();

    const wrapper = mount(LibraryView, {
      attachTo: document.body,
      global: {
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find("[data-entry-key='file:file-1']").exists()).toBe(true);
      expect(router.currentRoute.value.query.file).toBeUndefined();
    });

    const vm = wrapper.vm as unknown as {
      explorerEntries: ExplorerEntry[];
      handleExplorerRowContextMenu: (event: { originalEvent: Event; data: ExplorerEntry }) => void;
      resourceMenuItems: TestLibraryMenuItem[];
    };
    const fileEntry = vm.explorerEntries.find((entry) => entry.kind === "file" && entry.id === "file-1");
    expect(fileEntry).toBeTruthy();

    vm.handleExplorerRowContextMenu({
      originalEvent: new MouseEvent("contextmenu", { bubbles: true }),
      data: fileEntry!,
    });
    vm.resourceMenuItems[0].onSelect();

    await vi.waitFor(() => {
      expect(router.currentRoute.value.query.file).toBe("file-1");
      expect(getLibraryFile).toHaveBeenCalledWith("file-1", expect.any(Object));
      const previewDialog = document.body.querySelector(".library-preview-dialog");
      expect(previewDialog).not.toBeNull();
      expect(previewDialog?.classList.contains("max-w-[min(96vw,72rem)]")).toBe(true);
      expect(document.body.textContent).toContain("Parsed report text");
    });

    wrapper.unmount();
  });

  it("keeps wide preview hidden until a file is selected", async () => {
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    vi.spyOn(apiClient, "getLibraryTree").mockResolvedValue(treeResponse as never);
    mockLibraryResources();
    vi.spyOn(apiClient, "getLibraryFile").mockResolvedValue(null as never);

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/library", name: "library", component: LibraryView }],
    });

    router.push("/library?folder=folder-1");
    await router.isReady();

    const wrapper = mount(LibraryView, {
      attachTo: document.body,
      global: {
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find("[data-entry-key='file:file-1']").exists()).toBe(true);
    });

    expect(wrapper.find(".library-docked-preview").exists()).toBe(false);

    const vm = wrapper.vm as unknown as {
      explorerEntries: ExplorerEntry[];
      handleExplorerRowDoubleClick: (event: { data: ExplorerEntry }) => void;
    };
    const fileEntry = vm.explorerEntries.find((entry) => entry.kind === "file" && entry.id === "file-1");
    expect(fileEntry).toBeTruthy();
    vm.handleExplorerRowDoubleClick({ data: fileEntry! });
    await vi.waitFor(() => {
      expect(router.currentRoute.value.query.file).toBe("file-1");
      expect(wrapper.find(".library-docked-preview").exists()).toBe(true);
    });

    wrapper.unmount();
  });
});
