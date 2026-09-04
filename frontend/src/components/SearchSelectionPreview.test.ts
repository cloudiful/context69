import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import SearchSelectionPreview from "./SearchSelectionPreview.vue";

function httpHit() {
  return {
    chunk_id: "chunk-http",
    document_id: 7,
    source_key: "gov_documents",
    external_id: "ext-http-7",
    group_key: "personal-admin",
    group_path: "personal-admin/default",
    visibility: "private",
    title: "Policy Circular",
    summary: "summary text",
    source_uri: "https://example.com/policy",
    published_at: "2025-01-02",
    chunk_index: 0,
    chunk_text: "important policy text excerpt",
    score: 0.87,
    metadata_json: {},
  };
}

function libraryHit() {
  return {
    chunk_id: "chunk-lib",
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
    chunk_text: "sheet content excerpt",
    score: 0.77,
    metadata_json: {},
    is_library_file: true,
    library_file_id: "file-2",
    library_path: "/",
    library_section_label: "Sim run 2024-01",
  };
}

describe("SearchSelectionPreview", () => {
  it("keeps title, excerpt, single http link and open for external hits", async () => {
    const hit = httpHit();
    const wrapper = mount(SearchSelectionPreview, {
      props: { selectedHit: hit as never },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const text = wrapper.text();
    expect(text).toContain("Policy Circular");
    expect(text).toContain("important policy text excerpt");
    expect(text).not.toContain("gov_documents");
    expect(text).not.toContain("ext-http-7");

    const links = wrapper.findAll("a");
    expect(links).toHaveLength(1);
    expect(links[0].attributes("href")).toBe("https://example.com/policy");

    await wrapper.get('[data-testid="search-preview-open"]').trigger("click");
    expect(wrapper.emitted("open")?.[0]).toEqual([expect.objectContaining({ chunk_id: "chunk-http" })]);
  });

  it("hides internal library fields and never links context69 URIs", async () => {
    const hit = libraryHit();
    const wrapper = mount(SearchSelectionPreview, {
      props: { selectedHit: hit as never },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const text = wrapper.text();
    expect(text).toContain("Budget Workbook");
    expect(text).toContain("sheet content excerpt");
    expect(text).not.toContain("file_library");
    expect(text).not.toContain("FILE_LIBRARY");
    expect(text).not.toContain("library-99");
    expect(text).not.toContain("Sim run 2024-01");
    expect(text).not.toContain("context69://library/files/file-2");
    expect(wrapper.findAll("a")).toHaveLength(0);
    expect(wrapper.find('a[href="context69://library/files/file-2"]').exists()).toBe(false);

    await wrapper.get('[data-testid="search-preview-open"]').trigger("click");
    expect(wrapper.emitted("open")?.[0]).toEqual([expect.objectContaining({ chunk_id: "chunk-lib" })]);
  });

  it("shows empty state without internal identifiers when nothing is selected", async () => {
    const wrapper = mount(SearchSelectionPreview, {
      props: { selectedHit: null },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    expect(wrapper.find('[data-testid="search-selection-preview"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("No Results");
    expect(wrapper.text()).not.toContain("file_library");
    expect(wrapper.findAll("a")).toHaveLength(0);
  });
});
