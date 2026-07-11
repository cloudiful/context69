import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryFolderNode, LibraryResourcePageResponse } from "../../services/api";
import { useProjectLibraryPage } from "./use-project-library-page";

const mocks = vi.hoisted(() => ({
  getGroupLibraryResources: vi.fn(),
  showErrorToast: vi.fn(),
}));

vi.mock("../../services/api", () => ({
  apiClient: { getGroupLibraryResources: mocks.getGroupLibraryResources },
}));

vi.mock("../use-error-toast", () => ({
  useErrorToast: () => mocks.showErrorToast,
}));

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
    page: 2,
    page_size: 25,
    total: 80,
    total_pages: 4,
  };
}

describe("useProjectLibraryPage", () => {
  beforeEach(() => {
    mocks.getGroupLibraryResources.mockReset();
    mocks.showErrorToast.mockReset();
  });

  it("loads a real backend page and forwards sorting parameters", async () => {
    mocks.getGroupLibraryResources.mockResolvedValue(response());
    const folder = ref<LibraryFolderNode | null>(root);
    const state = useProjectLibraryPage({ groupPath: "stock/alpha", folder, t: (key) => key });

    state.query.value = "latest";
    await state.changePage(25, 25);
    await state.changeSort("size", -1);

    expect(mocks.getGroupLibraryResources).toHaveBeenLastCalledWith("stock/alpha", {
      folderId: null,
      page: 1,
      pageSize: 25,
      query: "latest",
      sortBy: "size",
      sortDirection: "desc",
    });
    expect(state.entries.value[0]).toMatchObject({
      kind: "file",
      name: "latest.txt",
      sizeBytes: 2048,
    });
    expect(state.total.value).toBe(80);
  });
});
