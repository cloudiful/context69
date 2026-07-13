import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import SearchResultList from "./SearchResultList.vue";

describe("SearchResultList", () => {
  it("emits open for document hits", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        hits: [
          {
            chunk_id: "chunk-1",
            document_id: 42,
            source_key: "gov_documents",
            external_id: "ext-1",
            group_key: "personal-admin",
            group_path: "personal-admin/default",
            visibility: "private",
            title: "Cyber Policy Update",
            summary: "summary",
            source_uri: "https://example.com/doc",
            published_at: "2025-01-01",
            chunk_index: 0,
            chunk_text: "policy content",
            score: 0.91,
            metadata_json: {},
          },
        ],
      },
      global: {
        plugins: [testNuxtUiPlugin, createTestI18n()],
      },
    });

    await wrapper.get('[data-testid="search-result-open"]').trigger("click");
    expect(wrapper.emitted("open")?.[0]).toEqual([
      expect.objectContaining({
        document_id: 42,
        chunk_id: "chunk-1",
      }),
    ]);
  });

  it("renders library-backed hits and emits open", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        hits: [
          {
            chunk_id: "chunk-2",
            document_id: 99,
            source_key: "file_library",
            external_id: "library-99",
            group_key: "personal-admin",
            group_path: "personal-admin/default",
            visibility: "private",
            title: "Budget Workbook",
            summary: "summary",
            source_uri: "context69://library/files/file-2",
            published_at: null,
            chunk_index: 0,
            chunk_text: "sheet content",
            score: 0.77,
            metadata_json: {},
            is_library_file: true,
            library_file_id: "file-2",
            library_path: "/Finance/Budget",
            library_section_label: "Sheet A",
          },
        ],
      },
      global: {
        plugins: [testNuxtUiPlugin, createTestI18n()],
      },
    });

    expect(wrapper.text()).toContain("/Finance/Budget");
    expect(wrapper.text()).toContain("Budget Workbook");
    await wrapper.get('[data-testid="search-result-open"]').trigger("click");
    expect(wrapper.emitted("open")?.[0]).toEqual([
      expect.objectContaining({
        library_file_id: "file-2",
        chunk_id: "chunk-2",
      }),
    ]);
  });
});
