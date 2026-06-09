import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import SearchResultList from "./SearchResultList.vue";

describe("SearchResultList", () => {
  it("navigates to the document detail route", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        {
          path: "/search",
          name: "search",
          component: { template: "<div>search</div>" },
        },
        {
          path: "/documents/:id",
          name: "document",
          component: { template: "<div>document detail</div>" },
        },
      ],
    });

    router.push("/search");
    await router.isReady();

    const wrapper = mount(SearchResultList, {
      props: {
        hits: [
          {
            chunk_id: "chunk-1",
            document_id: 42,
            source_key: "gov_documents",
            external_id: "ext-1",
            group_key: "personal-admin",
            project_key: "default",
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
        plugins: [testPrimeVuePlugin, router, createTestI18n()],
      },
    });

    await wrapper.get('[data-testid="search-result-open"]').trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("document");
    expect(router.currentRoute.value.params.id).toBe("42");
  });

  it("routes library-backed hits to the library view", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        {
          path: "/search",
          name: "search",
          component: { template: "<div>search</div>" },
        },
        {
          path: "/library",
          name: "library",
          component: { template: "<div>library</div>" },
        },
      ],
    });

    router.push("/search");
    await router.isReady();

    const wrapper = mount(SearchResultList, {
      props: {
        hits: [
          {
            chunk_id: "chunk-2",
            document_id: 99,
            source_key: "file_library",
            external_id: "library-99",
            group_key: "personal-admin",
            project_key: "default",
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
        plugins: [testPrimeVuePlugin, router, createTestI18n()],
      },
    });

    expect(wrapper.text()).toContain("/Finance/Budget");
    expect(wrapper.find(".search-card-list").exists()).toBe(true);
    expect(wrapper.find(".tool-card").text()).toContain("Budget Workbook");
    await wrapper.get('[data-testid="search-result-open"]').trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("library");
    expect(router.currentRoute.value.query.file).toBe("file-2");
  });
});
