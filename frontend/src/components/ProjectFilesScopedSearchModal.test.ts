import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import ProjectFilesScopedSearchModal from "./ProjectFilesScopedSearchModal.vue";
import type { SearchHit } from "../services/api";

function createHit(overrides: Partial<SearchHit> = {}): SearchHit {
  return {
    chunk_id: "chunk-1",
    chunk_index: 0,
    chunk_text: "alpha beta",
    document_id: 1,
    external_id: "doc-1",
    group_key: "stock",
    group_path: "stock/reports",
    score: 0.9,
    source_key: "stock",
    source_uri: "s3://stock/report.md",
    title: "Report",
    visibility: "private",
    ...overrides,
  };
}

function mountModal(props: { loading?: boolean; results?: SearchHit[]; query?: string } = {}) {
  return mount(ProjectFilesScopedSearchModal, {
    props: {
      open: true,
      title: "Search Results in reports",
      loading: props.loading ?? false,
      query: props.query ?? "alpha",
      results: props.results ?? [],
    },
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n()],
      stubs: {
        Modal: { template: "<div><slot name=\"body\" /></div>" },
        MarkdownChunk: {
          props: ["content", "highlight"],
          template: "<div class=\"markdown-chunk\" :data-content=\"content\" :data-highlight=\"highlight\" />",
        },
      },
    },
  });
}

describe("ProjectFilesScopedSearchModal", () => {
  it("shows the searching state while a scoped search is running", () => {
    const wrapper = mountModal({ loading: true, results: [createHit()] });

    expect(wrapper.text()).toContain("Searching...");
    expect(wrapper.find('[data-testid="scoped-content-search-results"]').exists()).toBe(false);

    wrapper.unmount();
  });

  it("shows the empty state when the search returned no results", () => {
    const wrapper = mountModal({ results: [] });

    expect(wrapper.text()).toContain("No Results");
    expect(wrapper.text()).toContain("No matches in this folder.");
    expect(wrapper.find('[data-testid="scoped-content-search-results"]').exists()).toBe(false);

    wrapper.unmount();
  });

  it("renders each result hit with the query as the markdown highlight", () => {
    const wrapper = mountModal({
      query: "beta",
      results: [
        createHit({ chunk_id: "chunk-1", title: "Report", group_path: "stock/reports", chunk_text: "beta here" }),
        createHit({ chunk_id: "chunk-2", title: "Notes", group_path: "stock/notes", chunk_text: "second" }),
      ],
    });

    const list = wrapper.get('[data-testid="scoped-content-search-results"]');
    const items = list.findAll("li");

    expect(items).toHaveLength(2);
    expect(items[0].text()).toContain("Report");
    expect(items[0].text()).toContain("stock/reports");
    expect(items[1].text()).toContain("Notes");

    const chunks = list.findAll(".markdown-chunk");
    expect(chunks).toHaveLength(2);
    expect(chunks[0].attributes("data-content")).toBe("beta here");
    expect(chunks[0].attributes("data-highlight")).toBe("beta");

    wrapper.unmount();
  });
});
