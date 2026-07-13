import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";

const { getDocument, ApiError } = vi.hoisted(() => {
  class MockApiError extends Error {
    status: number;

    constructor(message: string, status: number) {
      super(message);
      this.name = "ApiError";
      this.status = status;
    }
  }

  return {
    getDocument: vi.fn(),
    ApiError: MockApiError,
  };
});

vi.mock("../services/api", () => ({
  apiClient: {
    getDocument,
  },
  ApiError,
}));

import DocumentView from "./DocumentView.vue";

async function mountView(path = "/documents/42") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/documents/:id", name: "document", component: DocumentView },
      { path: "/library", name: "library", component: { template: "<div />" } },
    ],
  });

  router.push(path);
  await router.isReady();

  return mount(DocumentView, {
    global: {
      plugins: [testPrimeVuePlugin, router, createTestI18n()],
    },
  });
}

describe("DocumentView", () => {
  beforeEach(() => {
    getDocument.mockReset();
  });

  it("renders a not found message for missing documents", async () => {
    getDocument.mockRejectedValue(new ApiError("Document missing", 404));

    const wrapper = await mountView();
    await flushPromises();

    expect(wrapper.text()).toContain("404");
    expect(wrapper.text()).toContain("Document missing");
  });

  it("renders document details when the request succeeds", async () => {
    getDocument.mockResolvedValue({
      document_id: 42,
      source_key: "gov_documents",
      external_id: "ext-42",
      title: "Cyber Policy Update",
      summary: "summary text",
      source_uri: "https://example.com/doc",
      published_at: "2025-01-01",
      updated_at: "2025-01-02T00:00:00Z",
      metadata_json: { category: "policy" },
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
});
