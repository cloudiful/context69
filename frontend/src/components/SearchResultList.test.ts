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

  it("hides library internals and never renders library URIs as links", async () => {
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
            library_path: "/",
            library_section_label: "Sim run 2024-01",
          },
        ],
      },
      global: {
        plugins: [testNuxtUiPlugin, createTestI18n()],
      },
    });

    const text = wrapper.text();
    expect(text).toContain("Budget Workbook");
    expect(text).toContain("sheet content");
    expect(text).not.toContain("Sim run 2024-01");
    expect(text).not.toContain("file_library");
    expect(text).not.toContain("library-99");
    expect(text).not.toContain("context69://library/files/file-2");
    expect(wrapper.findAll("a")).toHaveLength(0);
    expect(wrapper.find('a[href="context69://library/files/file-2"]').exists()).toBe(false);
    await wrapper.get('[data-testid="search-result-open"]').trigger("click");
    expect(wrapper.emitted("open")?.[0]).toEqual([
      expect.objectContaining({
        library_file_id: "file-2",
        chunk_id: "chunk-2",
      }),
    ]);
  });

  it("keeps a single clickable http source action without raw URL text or internal badges", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 1, total_pages: 1 },
        hits: [
          {
            chunk_id: "chunk-http",
            document_id: 21,
            source_key: "gov_documents",
            external_id: "ext-http-1",
            group_key: "g",
            group_path: "g/p",
            visibility: "private",
            title: "External Doc",
            summary: "",
            source_uri: "https://example.com/external-doc",
            published_at: "2025-01-01",
            chunk_index: 0,
            chunk_text: "external content excerpt",
            score: 0.82,
            metadata_json: {},
          },
        ],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const text = wrapper.text();
    expect(text).toContain("External Doc");
    expect(text).not.toContain("gov_documents");
    expect(text).not.toContain("ext-http-1");
    // The raw full-URL row was replaced by an icon action; the URL is not body text.
    expect(text).not.toContain("https://example.com/external-doc");
    const links = wrapper.findAll("a");
    expect(links).toHaveLength(1);
    expect(links[0].attributes("href")).toBe("https://example.com/external-doc");
    expect(links[0].attributes("target")).toBe("_blank");
    expect(links[0].attributes("rel")).toBe("noopener");
    expect(links[0].attributes("title")).toBe("https://example.com/external-doc");
    expect(links[0].attributes("aria-label")).toBe("Open source link");
    expect(links[0].find("[data-slot='leadingIcon']").exists()).toBe(true);
  });

  it("keeps long source URLs reachable through the external action with an accessible label", async () => {
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
    const link = wrapper.find('a[href="' + longUrl + '"]');
    expect(link.exists()).toBe(true);
    expect(link.attributes("title")).toBe(longUrl);
    expect(link.attributes("aria-label")).toBe("Open source link");
    // No truncated URL row is rendered as text anymore.
    expect(wrapper.text()).not.toContain("a".repeat(50));
    expect(wrapper.text()).not.toContain("[object Object]");
    expect(wrapper.text()).not.toContain("ext-long");
  });

  it("selects via title link without ghost hover and keeps aria-selected", async () => {
    const hits = [
      {
        chunk_id: "chunk-a",
        document_id: 1,
        source_key: "src",
        external_id: "ext-a",
        group_key: "g",
        group_path: "g/p",
        visibility: "private" as const,
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
        visibility: "private" as const,
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

    const selectButtons = wrapper.findAll('[data-testid="search-result-select"]');
    expect(selectButtons).toHaveLength(2);
    expect(selectButtons[0].attributes("aria-selected")).toBe("false");
    expect(selectButtons[1].attributes("aria-selected")).toBe("true");
    const titleComponents = wrapper.findAllComponents({ name: "UButton" }).filter((node) => node.attributes("data-testid") === "search-result-select");
    if (titleComponents.length > 0) {
      expect(titleComponents[0].props("variant")).toBe("link");
    } else {
      // Fallback when stubbed: link variant must not carry ghost fill hover.
      expect(selectButtons[0].html()).not.toContain("hover:bg");
    }

    await selectButtons[0].trigger("click");
    expect(wrapper.emitted("preview")?.[0]?.[0]).toEqual(expect.objectContaining({ chunk_id: "chunk-a" }));

    // inline preview and open actions are distinct; open targets the document
    await wrapper.findAll('[data-testid="search-result-preview"]')[0].trigger("click");
    expect(wrapper.emitted("preview")?.[0]?.[0]).toEqual(expect.objectContaining({ chunk_id: "chunk-a" }));
    await wrapper.findAll('[data-testid="search-result-open"]')[0].trigger("click");
    expect(wrapper.emitted("open")?.[0]?.[0]).toEqual(expect.objectContaining({ chunk_id: "chunk-a" }));
  });

  it("uses fill-height scroll without capped max-height and keeps pagination", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 20, total_pages: 3 },
        hits: [],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    const scroll = wrapper.get('[data-testid="search-results-scroll"]');
    const scrollClass = ` ${(scroll.attributes("class") ?? "").replace(/\s+/g, " ")} `;
    expect(scrollClass).toContain(" h-full ");
    expect(scrollClass).toMatch(/ min-h-/);
    expect(scrollClass).toContain(" overflow-y-auto ");
    expect(wrapper.html()).not.toContain("min(56vh");
    expect(wrapper.html()).not.toContain("max-h-[min(56vh");
    // Cursor-style pagination stays mounted for bounded navigation.
    expect(wrapper.find('[data-testid="search-prev"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="search-next"]').exists()).toBe(true);
  });

  it("keeps title link focus outline without ghost hover fill", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 1, page_size: 8, total: 1, total_pages: 1 },
        hits: [
          {
            chunk_id: "chunk-focus",
            document_id: 3,
            source_key: "src",
            external_id: "ext-focus",
            group_key: "g",
            group_path: "g/p",
            visibility: "private",
            title: "Focus Doc",
            summary: "",
            source_uri: "https://example.com/focus",
            published_at: "2025-02-01",
            chunk_index: 0,
            chunk_text: "focus content",
            score: 0.6,
            metadata_json: {},
          },
        ],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    // Scholar-density rows are rendered as a list, not a UTable.
    expect(wrapper.find("tbody").exists()).toBe(false);
    expect(wrapper.findAll('[data-testid="search-result-item"]')).toHaveLength(1);
    const titleButton = wrapper.get('[data-testid="search-result-select"]');
    // The title link must not carry a ghost hover fill; focus outline stays native.
    expect(titleButton.html()).not.toContain("hover:bg");
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

  it("navigates with ordering-epoch cursors and keeps compact page size options", async () => {
    const wrapper = mount(SearchResultList, {
      props: {
        pagination: { page: 2, page_size: 8, total: 20, total_pages: 3, has_more: true, next_cursor: "c1:16", prev_cursor: "c1:0" },
        hits: [],
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    // pageSizeOptions computed includes 8 through the page-size select.
    expect(wrapper.find('[data-testid="search-page-size"]').exists()).toBe(true);
    await wrapper.get('[data-testid="search-prev"]').trigger("click");
    expect(wrapper.emitted("prev")).toBeTruthy();
    await wrapper.get('[data-testid="search-next"]').trigger("click");
    expect(wrapper.emitted("next")).toBeTruthy();
  });
});
