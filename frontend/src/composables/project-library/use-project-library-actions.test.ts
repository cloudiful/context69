import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryFolderNode, LibraryIngestJobResponse } from "../../services/api";
import { useProjectLibraryActions } from "./use-project-library-actions";

const mocks = vi.hoisted(() => ({
  getGroupLibraryFile: vi.fn(),
  retryGroupLibraryFile: vi.fn(),
  showErrorToast: vi.fn(),
  toastAdd: vi.fn(),
}));

vi.mock("../../services/api", () => ({
  apiClient: {
    getGroupLibraryFile: mocks.getGroupLibraryFile,
    retryGroupLibraryFile: mocks.retryGroupLibraryFile,
  },
}));
vi.mock("../use-error-toast", () => ({ useErrorToast: () => mocks.showErrorToast }));
vi.mock("../use-app-confirm", () => ({ useAppConfirm: () => ({ require: vi.fn() }) }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.toastAdd }) }));

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
    mocks.retryGroupLibraryFile.mockReset();
    mocks.getGroupLibraryFile.mockReset();
    mocks.getGroupLibraryFile.mockResolvedValue({ source_available: true });
    mocks.showErrorToast.mockReset();
    mocks.toastAdd.mockReset();
  });

  it("prevents duplicate retry requests and starts job polling", async () => {
    let resolveJob: ((job: LibraryIngestJobResponse) => void) | undefined;
    mocks.retryGroupLibraryFile.mockImplementation(() => new Promise((resolve) => {
      resolveJob = resolve;
    }));
    const loadTree = vi.fn().mockResolvedValue(undefined);
    const schedulePolling = vi.fn();
    const groupPath = ref("group-a");
    const state = useProjectLibraryActions({
      groupPath,
      loadTree,
      moveOptions: ref([]),
      replaceSelection: vi.fn().mockResolvedValue(undefined),
      schedulePolling,
      selectFile: vi.fn().mockResolvedValue(undefined),
      selectedFolder: ref(root),
      selectedFileId: ref("file-id"),
      t: (key) => key,
      updateExpandedForFolder: vi.fn(),
      previewDocked: ref(false),
      previewDialogVisible: ref(false),
    });

    groupPath.value = "group-b";
    const first = state.retryFile("file-id");
    await state.retryFile("file-id");
    expect(mocks.retryGroupLibraryFile).toHaveBeenCalledTimes(1);
    expect(mocks.retryGroupLibraryFile).toHaveBeenCalledWith("group-b", "file-id");
    expect(state.retryingFileIds.value).toEqual(["file-id"]);

    resolveJob?.({ job_id: "job-id" } as LibraryIngestJobResponse);
    await first;

    expect(loadTree).toHaveBeenCalledOnce();
    expect(schedulePolling).toHaveBeenCalledWith(["job-id"]);
    expect(state.retryingFileIds.value).toEqual([]);
  });

  it("does not retry when the stored original is missing", async () => {
    mocks.getGroupLibraryFile.mockResolvedValue({ source_available: false });
    const state = useProjectLibraryActions({
      groupPath: ref("group"),
      loadTree: vi.fn(),
      moveOptions: ref([]),
      replaceSelection: vi.fn(),
      schedulePolling: vi.fn(),
      selectFile: vi.fn(),
      selectedFolder: ref(root),
      selectedFileId: ref("file-id"),
      t: (key) => key,
      updateExpandedForFolder: vi.fn(),
      previewDocked: ref(false),
      previewDialogVisible: ref(false),
    });

    await state.retryFile("file-id");

    expect(mocks.retryGroupLibraryFile).not.toHaveBeenCalled();
    expect(state.unavailableFileIds.value).toEqual(["file-id"]);
    expect(mocks.showErrorToast).toHaveBeenCalledOnce();
  });
});
