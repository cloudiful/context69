import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import type { FolderSummary } from "../types/library";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import ProjectFilesPreviewModal from "./ProjectFilesPreviewModal.vue";

const PreviewPanelStub = {
  name: "LibraryPreviewPanel",
  props: [
    "activeSectionKey",
    "detail",
    "detailLoading",
    "groupPath",
    "selectedFileId",
    "selectedFolderSummary",
    "releasable",
    "releasing",
    "retrying",
  ],
  emits: ["retry", "release", "update:activeSectionKey"],
  template: "<div class=\"preview-panel-stub\" />",
};

const ModalStub = {
  name: "Modal",
  props: ["open", "title"],
  emits: ["update:open"],
  template: "<div class=\"modal-stub\"><slot name=\"body\" /></div>",
};

type PreviewModalOverrides = {
  open?: boolean;
  activeSectionKey?: string;
  detailLoading?: boolean;
  selectedFileId?: string | null;
  selectedFolderSummary?: FolderSummary | null;
  releasable?: boolean;
  releasing?: boolean;
  retrying?: boolean;
};

function mountModal(overrides: PreviewModalOverrides = {}) {
  return mount(ProjectFilesPreviewModal, {
    props: {
      open: overrides.open ?? true,
      title: "report.md",
      activeSectionKey: overrides.activeSectionKey ?? "document",
      detail: null,
      detailLoading: overrides.detailLoading ?? false,
      groupPath: "stock/reports",
      selectedFileId: overrides.selectedFileId ?? null,
      selectedFolderSummary: overrides.selectedFolderSummary ?? null,
      releasable: overrides.releasable ?? false,
      releasing: overrides.releasing ?? false,
      retrying: overrides.retrying ?? false,
    },
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n()],
      stubs: {
        Modal: ModalStub,
        LibraryPreviewPanel: PreviewPanelStub,
      },
    },
  });
}

describe("ProjectFilesPreviewModal", () => {
  it("keeps the preview modal sizing and round-trips the open model", async () => {
    const wrapper = mountModal();
    const modal = wrapper.getComponent({ name: "Modal" });

    expect(modal.props("open")).toBe(true);
    expect(modal.props("title")).toBe("report.md");
    expect(modal.classes()).toContain("library-preview-dialog");
    expect(modal.classes()).toContain("w-[min(96vw,72rem)]");
    expect(modal.classes()).toContain("max-w-[min(96vw,72rem)]");

    await modal.vm.$emit("update:open", false);

    expect(wrapper.emitted("update:open")).toEqual([[false]]);
    wrapper.unmount();
  });

  it("forwards the preview selection and round-trips the active section", async () => {
    const wrapper = mountModal({
      activeSectionKey: "summary",
      detailLoading: true,
      selectedFileId: "file-1",
      selectedFolderSummary: {
        childFolderCount: 0,
        fileCount: 1,
        isSourceFolder: false,
        isSourceRecordsFolder: false,
        name: "reports",
        path: "stock/reports",
        processingCount: 0,
      },
    });
    const panel = wrapper.getComponent({ name: "LibraryPreviewPanel" });

    expect(panel.props("activeSectionKey")).toBe("summary");
    expect(panel.props("groupPath")).toBe("stock/reports");
    expect(panel.props("detailLoading")).toBe(true);
    expect(panel.props("selectedFileId")).toBe("file-1");
    expect(panel.props("selectedFolderSummary")).toMatchObject({ name: "reports" });

    await panel.vm.$emit("update:activeSectionKey", "document");

    expect(wrapper.emitted("update:activeSectionKey")).toEqual([["document"]]);
    wrapper.unmount();
  });

  it("forwards retry and release with the action flags", async () => {
    const wrapper = mountModal({ releasable: true, releasing: true, retrying: true });
    const panel = wrapper.getComponent({ name: "LibraryPreviewPanel" });

    expect(panel.props("releasable")).toBe(true);
    expect(panel.props("releasing")).toBe(true);
    expect(panel.props("retrying")).toBe(true);

    await panel.vm.$emit("retry", "file-1");
    await panel.vm.$emit("release", "file-2");

    expect(wrapper.emitted("retry")).toEqual([["file-1"]]);
    expect(wrapper.emitted("release")).toEqual([["file-2"]]);
    wrapper.unmount();
  });
});
