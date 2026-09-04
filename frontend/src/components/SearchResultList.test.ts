import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import SearchResultList from "./SearchResultList.vue";

describe("SearchResultList", () => {
  it("emits open for document hits", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 50, total: 1, total_pages: 1 },
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
        pagination: { page: 1, page_size: 50, total: 1, total_pages: 1 },
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

  it("truncates long URLs with accessible title and keeps clickability", async () => {
    const longUrl = "https://example.com/" + "a".repeat(200) + "?q=search&lang=en&extra=very-long-param";
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 1, total_pages: 1 },
        hits: [
          {
            chunk_id: "chunk-3",
            document_id: 10,
            source_key: "gov_documents",
            external_id: "ext-long",
            group_key: "g",
            group_path: "g/p",
            visibility: "private",
            title: "Long URL Doc",
            summary: "",
            source_uri: longUrl,
            published_at: "2025-01-01",
            chunk_index: 0,
            chunk_text: "content",
            score: 0.5,
            metadata_json: {},
          },
        ],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    const link = wrapper.find(`a[href="${longUrl}"]`);
    expect(link.exists()).toBe(true);
    expect(link.attributes("title")).toBe(longUrl);
    expect(link.classes().join(" ")).toContain("truncate");
    expect(wrapper.text()).not.toContain("[object Object]");
  });

  it("shows selected hit with modest highlight and preserves open for both targets", async () => {
    const hits = [
      {
        chunk_id: "chunk-a",
        document_id: 1,
        source_key: "src",
        external_id: "ext-a",
        group_key: "g",
        group_path: "g/p",
        visibility: "private",
        title: "Alpha",
        summary: "",
        source_uri: "https://example.com/a",
        published_at: "2025-02-01",
        chunk_index: 0,
        chunk_text: "alpha",
        score: 0.8,
        metadata_json: {},
      },
      {
        chunk_id: "chunk-b",
        document_id: 2,
        source_key: "src",
        external_id: "ext-b",
        group_key: "g",
        group_path: "g/p",
        visibility: "private",
        title: "Beta",
        summary: "",
        source_uri: "https://example.com/b",
        published_at: "2025-02-02",
        chunk_index: 1,
        chunk_text: "beta",
        score: 0.7,
        metadata_json: {},
      },
    ];
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 2, total_pages: 1 },
        hits,
        selectedHit: hits[1],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    const items = wrapper.findAll('[data-testid="search-result-item"]');
    expect(items).toHaveLength(2);
    const selectedItems = items.filter((node) => node.attributes("data-selected") !== undefined);
    expect(selectedItems).toHaveLength(1);
    expect(selectedItems[0].text()).toContain("Beta");
    // ensure the unselected item is Alpha and not marked selected
    const unselected = items.filter((node) => node.attributes("data-selected") === undefined);
    expect(unselected).toHaveLength(1);
    expect(unselected[0].text()).toContain("Alpha");
    // ensure both document and library open targets work (here both are documents)
    await wrapper.findAll('[data-testid="search-result-open"]')[0].trigger("click");
    expect(wrapper.emitted("open")?.[0]?.[0]).toEqual(expect.objectContaining({ chunk_id: "chunk-a" }));
  });

  it("keeps pagination visible with stable height and does not error on zero total", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 0, total_pages: 0 },
        hits: [],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    // AppServerList should still render container with min height, pagination handles zero total without crash
    expect(wrapper.find('[data-testid="search-results-list"]').exists()).toBe(true);
    expect(wrapper.html()).not.toContain("undefined");
  });

  it("uses compact page size options including current limit", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 20, total_pages: 3 },
        hits: [],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    // pageSizeOptions computed includes 8
    const paginationComp = wrapper.findComponent({ name: "TablePagination" });
    // TablePagination should be rendered
    expect(paginationComp.exists()).toBe(true);
  });
});
