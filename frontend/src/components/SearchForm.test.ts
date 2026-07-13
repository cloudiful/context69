import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import Select from "@nuxt/ui/components/Select.vue";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import type { SearchFilters } from "../types/ui";
import SearchForm from "./SearchForm.vue";

describe("SearchForm", () => {
  it("emits filter updates and submit", async () => {
    const wrapper = mount(SearchForm, {
      props: {
        filters: {
          query: "",
          sourceKey: "",
          publishedAfter: "",
          publishedBefore: "",
          limit: 8,
        },
        historyEntries: [],
        sources: [
          {
            source_key: "gov_documents",
            group_key: "personal-admin",
            group_path: "personal-admin/default",
            visibility: "private",
            display_name: "国务院/部委政策公文",
            description: "覆盖国务院及部委正式政策公文。",
            example_queries: ["新能源汽车 购置税 政策"],
            connection: "gov-info",
            has_database_url: true,
            origin_status: "connected",
            origin_message: null,
            sync_strategy: "cursor",
            connector_type: "postgres_sql",
            base_query: "SELECT 1",
            batch_size: 200,
            last_cursor_updated_at: null,
            last_cursor_external_id: null,
            last_success_at: null,
          },
        ],
        busy: false,
      },
      global: {
        plugins: [testNuxtUiPlugin, createTestI18n()],
      },
    });

    expect(wrapper.get("#query").attributes("placeholder")).toBe("Query");

    await wrapper.get("#query").setValue("cybersecurity");
    await wrapper.get("[data-testid='search-toggle-advanced']").trigger("click");
    await wrapper.findComponent({ name: "Select" }).vm.$emit("update:modelValue", "gov_documents");
    await wrapper.get("form").trigger("submit");

    const updates = wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined;
    expect(updates?.some(([filters]) => filters.query === "cybersecurity")).toBe(true);
    expect(updates?.some(([filters]) => filters.sourceKey === "gov_documents")).toBe(true);
    expect(wrapper.emitted("submit")).toHaveLength(1);
  });

  it("keeps advanced filters collapsed by default and auto-opens when date filters exist", async () => {
    const wrapper = mount(SearchForm, {
      props: {
        filters: {
          query: "",
          sourceKey: "",
          publishedAfter: "",
          publishedBefore: "",
          limit: 8,
        },
        historyEntries: [],
        sources: [],
        busy: false,
      },
      global: {
        plugins: [testNuxtUiPlugin, createTestI18n()],
      },
    });

    expect(wrapper.find("[data-testid='search-published-after']").exists()).toBe(false);

    await wrapper.setProps({
      filters: {
        query: "",
        sourceKey: "",
        publishedAfter: "2025-01-01",
        publishedBefore: "",
        limit: 8,
      },
    });

    expect(wrapper.find("[data-testid='search-published-after']").exists()).toBe(true);

    await wrapper.get("[data-testid='search-reset']").trigger("click");

    const lastUpdate = wrapper.emitted("update:filters")?.at(-1)?.[0];
    expect(lastUpdate).toEqual({
      query: "",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    });
    expect(wrapper.find("[data-testid='search-published-after']").exists()).toBe(false);
  });
});
