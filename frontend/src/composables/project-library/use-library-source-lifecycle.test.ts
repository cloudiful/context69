import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { useI18n } from "vue-i18n";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import type { ExplorerEntry } from "../../types/library";
import { useLibrarySourceLifecycle } from "./use-library-source-lifecycle";

const confirmMocks = vi.hoisted(() => ({ require: vi.fn() }));
vi.mock("../use-app-confirm", () => ({
  useAppConfirm: () => ({ require: confirmMocks.require }),
}));

const getGroupLibraryFile = vi.spyOn(apiClient, "getGroupLibraryFile");
const releaseGroupLibraryFileSource = vi.spyOn(apiClient, "releaseGroupLibraryFileSource");
const submitTask = vi.spyOn(apiClient, "submitTask");
const getTask = vi.spyOn(apiClient, "getTask");

function fileEntry(overrides: Partial<Extract<ExplorerEntry, { kind: "file" }>> = {}): ExplorerEntry {
  return {
    key: "file:f1",
    kind: "file",
    id: "f1",
    depth: 0,
    name: "doc.txt",
    parentFolderId: null,
    path: "/",
    updatedAt: null,
    mediaType: "text/plain",
    sizeBytes: 10,
    ingestStatus: "succeeded",
    errorMessage: null,
    isSourceConfigFile: false,
    isSourceRecordFile: false,
    file: {} as never,
    ...overrides,
  };
}

interface ConfirmOptions {
  message: string;
  acceptLabel?: string;
  accept?: () => void | Promise<void>;
}

function setup() {
  let state!: ReturnType<typeof useLibrarySourceLifecycle>;
  const loadTree = vi.fn().mockResolvedValue(undefined);
  mount(defineComponent({
    setup() {
      const { t } = useI18n();
      state = useLibrarySourceLifecycle({ groupPath: ref("group"), loadTree, t });
      return {};
    },
    template: "<div />",
  }), { global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] } });
  return { state, loadTree };
}

describe("useLibrarySourceLifecycle", () => {
  beforeEach(() => {
    confirmMocks.require.mockReset();
    getGroupLibraryFile.mockReset();
    releaseGroupLibraryFileSource.mockReset();
    submitTask.mockReset();
    getTask.mockReset();
    getTask.mockResolvedValue({ task_id: "task-id", status: "succeeded" } as never);
  });

  it("offers retry for failed and stale pending/running files only", () => {
    const { state } = setup();

    for (const status of ["pending", "running", "failed", "cancelled"] as const) {
      expect(state.canRetry(fileEntry({ ingestStatus: status })), status).toBe(true);
    }
    expect(state.canRetry(fileEntry({ ingestStatus: "succeeded" }))).toBe(false);
  });

  it("does not retry when the source is unavailable", async () => {
    getGroupLibraryFile.mockResolvedValue({ file_id: "f1", source_available: false } as never);
    const { state } = setup();

    await state.retryFile("f1");

    expect(submitTask).not.toHaveBeenCalled();
    expect(state.unavailableFileIds.value).toEqual(["f1"]);
    expect(state.canRetry(fileEntry({ ingestStatus: "failed" }))).toBe(false);
  });

  it("requires confirmation before releasing the original file", async () => {
    releaseGroupLibraryFileSource.mockResolvedValue({ file_id: "f1", source_available: false } as never);
    let captured: ConfirmOptions | undefined;
    confirmMocks.require.mockImplementation((options: ConfirmOptions) => { captured = options; });
    const { state, loadTree } = setup();

    state.releaseFileSource("f1", "doc.txt");

    expect(captured).toBeTruthy();
    expect(captured!.message).toContain("doc.txt");
    expect(captured!.message).toContain("cannot be undone");
    expect(captured!.message).toContain("kept");
    expect(captured!.acceptLabel).toBe("Release original file");
    expect(releaseGroupLibraryFileSource).not.toHaveBeenCalled();

    await captured!.accept?.();
    await flushPromises();

    expect(releaseGroupLibraryFileSource).toHaveBeenCalledWith("group", "f1");
    expect(loadTree).toHaveBeenCalledOnce();
    expect(state.unavailableFileIds.value).toEqual(["f1"]);
    expect(state.canReleaseSource(fileEntry())).toBe(false);
  });

  it("only allows releasing a completed, user-owned file with an available source", () => {
    const { state } = setup();

    expect(state.canReleaseSource(fileEntry())).toBe(true);
    expect(state.canReleaseSource(fileEntry({ ingestStatus: "failed" }))).toBe(false);
    expect(state.canReleaseSource(fileEntry({ isSourceConfigFile: true }))).toBe(false);

    state.observeSourceAvailability({ file_id: "f1", source_available: false });
    expect(state.canReleaseSource(fileEntry())).toBe(false);

    state.observeSourceAvailability({ file_id: "f1", source_available: true });
    expect(state.canReleaseSource(fileEntry())).toBe(true);
  });
});
