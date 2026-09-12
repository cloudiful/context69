import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import type { LibraryFileDetailResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import LibraryPreviewPanel from "./LibraryPreviewPanel.vue";

const detail: LibraryFileDetailResponse = {
  created_at: "2026-07-12T04:11:00Z",
  file_id: "00000000-0000-0000-0000-000000000001",
  filename: "report.md",
  folder_path: "reports",
  group_key: "stock",
  group_path: "stock/disclosures",
  ingest_status: "succeeded",
  ingested_at: "2026-07-12T04:11:00Z",
  media_type: "text/markdown",
  sections: [{
    content_format: "plain_text",
    document_id: 6655,
    preview_text: "Body only",
    section_key: "document",
    section_label: "report",
    sort_order: 0,
    title: "Duplicate title",
  }],
  sha256: "abc",
  size_bytes: 4915,
  source_available: true,
  updated_at: "2026-07-12T04:11:00Z",
  visibility: "private",
};

function mountPreview(overrides: Partial<LibraryFileDetailResponse> = {}) {
  return mount(LibraryPreviewPanel, {
    props: {
      activeSectionKey: "document",
      detail: { ...detail, ...overrides },
      detailLoading: false,
      selectedFileId: detail.file_id,
      selectedFolderSummary: null,
    },
    global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
  });
}

function findRetryButton(wrapper: ReturnType<typeof mountPreview>) {
  return wrapper.findAll("button").find((button) => button.text().includes("Retry"));
}

describe("LibraryPreviewPanel", () => {
  it("shows single-section content without repeating its title", () => {
    const wrapper = mount(LibraryPreviewPanel, {
      props: {
        activeSectionKey: "document",
        detail,
        detailLoading: false,
        selectedFileId: detail.file_id,
        selectedFolderSummary: null,
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n("zh-CN")] },
    });

    expect(wrapper.text()).toContain("Body only");
    expect(wrapper.text()).not.toContain("Duplicate title");
    expect(wrapper.text()).not.toContain("文档 #6655");
  });

  it("offers retry for stale pending/running files and emits the file id", async () => {
    for (const ingestStatus of ["pending", "running"] as const) {
      const wrapper = mountPreview({ ingest_status: ingestStatus, source_available: true });
      const retry = findRetryButton(wrapper);

      expect(retry, ingestStatus).toBeDefined();
      await retry!.trigger("click");
      expect(wrapper.emitted("retry")?.[0]).toEqual([detail.file_id]);
      wrapper.unmount();
    }
  });

  it("hides retry and warns when the source is missing", () => {
    const wrapper = mountPreview({ ingest_status: "pending", source_available: false });

    expect(wrapper.text()).toContain("Original file is missing");
    expect(findRetryButton(wrapper)).toBeUndefined();
    wrapper.unmount();
  });
});
