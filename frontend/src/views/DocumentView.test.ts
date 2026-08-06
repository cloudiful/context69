import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { ApiError, apiClient } from "../services/api";
import DocumentView from "./DocumentView.vue";

const getDocument = vi.spyOn(apiClient, "getDocument");

async function mountView(path = "/documents/42") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/documents/:id", name: "document", component: DocumentView },
      { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div />" } },
    ],
  });

  router.push(path);
  await router.isReady();

  return mount(DocumentView, {
    global: {
      plugins: [testNuxtUiPlugin, router, createTestI18n()],
    },
  });
}

describe("DocumentView", () => {
  beforeEach(() => {
    getDocument.mockReset();
  });

  it("renders a not found message for missing documents", async () => {
    getDocument.mockImplementation(() => Promise.reject(new ApiError("Document missing", 404)));

    const wrapper = await mountView();
    await flushPromises();

    expect(wrapper.text()).toContain("404");
    expect(wrapper.text()).toContain("Document missing");
  });

  it("renders document details when the request succeeds", async () => {
    getDocument.mockResolvedValue({
      document_id: 42,
      source_key: "gov_documents",
      group_key: "personal-admin",
      group_path: "personal-admin/default",
      external_id: "ext-42",
      record_hash: "hash-42",
      visibility: "private",
      title: "Cyber Policy Update",
      summary: "summary text",
      source_uri: "https://example.com/doc",
      published_at: "2025-01-01",
      updated_at: "2025-01-02T00:00:00Z",
      metadata_json: {},
      library_path: "/Policies/Cyber",
      library_section_label: "Summary",
      is_library_file: true,
      library_file_id: "file-42",
      chunks: [
        {
          chunk_id: "chunk-1",
          chunk_index: 0,
          text: "First chunk",
        },
      ],
    });

    const wrapper = await mountView();
    await flushPromises();

    expect(wrapper.text()).toContain("Cyber Policy Update");
    expect(wrapper.text()).toContain("First chunk");
    expect(wrapper.text()).toContain("/Policies/Cyber");
  });

  it("paginates content blocks ten per page", async () => {
    getDocument.mockResolvedValue({
      document_id: 42,
      source_key: "gov_documents",
      group_key: "personal-admin",
      group_path: "personal-admin/default",
      external_id: "ext-42",
      record_hash: "hash-42",
      visibility: "private",
      title: "Cyber Policy Update",
      summary: "summary text",
      source_uri: "https://example.com/doc",
      published_at: "2025-01-01",
      updated_at: "2025-01-02T00:00:00Z",
      metadata_json: {},
      library_path: null,
      library_section_label: null,
      is_library_file: false,
      library_file_id: null,
      chunks: Array.from({ length: 12 }, (_, index) => ({
        chunk_id: `chunk-${index + 1}`,
        chunk_index: index,
        text: `block-${index + 1}`,
      })),
    });

    const wrapper = await mountView();
    await flushPromises();
    const chunkTexts = () => wrapper.findAll("pre").map((node) => node.text());

    expect(chunkTexts()).toContain("block-1");
    expect(chunkTexts()).toContain("block-10");
    expect(chunkTexts()).not.toContain("block-11");
    expect(wrapper.findComponent({ name: "Pagination" }).props("itemsPerPage")).toBe(10);

    wrapper.findComponent({ name: "Pagination" }).vm.$emit("update:page", 2);
    await wrapper.vm.$nextTick();

    expect(chunkTexts()).not.toContain("block-1");
    expect(chunkTexts()).toContain("block-11");
    expect(chunkTexts()).toContain("block-12");
  });
});
