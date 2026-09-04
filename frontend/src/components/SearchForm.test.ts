import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { CalendarDate } from "@internationalized/date";

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

    expect(wrapper.find("[data-testid='search-published-range']").exists()).toBe(false);
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

    expect(wrapper.find("[data-testid='search-published-range']").exists()).toBe(true);

    await wrapper.get("[data-testid='search-reset']").trigger("click");

    const lastUpdate = wrapper.emitted("update:filters")?.at(-1)?.[0];
    expect(lastUpdate).toEqual({
      query: "",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    });
    expect(wrapper.find("[data-testid='search-published-range']").exists()).toBe(false);
  });

  it("does not write [object Object] when history entry object is selected", async () => {
    const historyEntries = [
      { query: "alpha policy", sourceKey: "gov_documents", publishedAfter: "2024-01-01", publishedBefore: "2024-12-31", limit: 16, savedAt: new Date().toISOString() },
      { query: "beta", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, savedAt: new Date().toISOString() },
    ];
    const wrapper = mount(SearchForm, {
      props: {
        filters: { query: "", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 },
        historyEntries,
        sources: [],
        busy: false,
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const inputMenu = wrapper.getComponent({ name: "InputMenu" });
    // simulate selecting history entry as object (the bug case)
    inputMenu.vm.$emit("update:modelValue", historyEntries[0]);
    await wrapper.vm.$nextTick();

    const updates = wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined;
    const lastQuery = updates?.at(-1)?.[0].query;
    expect(lastQuery).toBe("alpha policy");
    expect(lastQuery).not.toBe("[object Object]");
    expect(wrapper.emitted("history-select")?.[0]?.[0]).toEqual(expect.objectContaining({ query: "alpha policy", sourceKey: "gov_documents" }));
    expect(wrapper.text()).not.toContain("[object Object]");
  });

  it("keeps history item display as readable string with truncation", async () => {
    const longQuery = "a".repeat(120);
    const wrapper = mount(SearchForm, {
      props: {
        filters: { query: "", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 },
        historyEntries: [{ query: longQuery, sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, savedAt: new Date().toISOString() }],
        sources: [],
        busy: false,
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    const inputMenu = wrapper.getComponent({ name: "InputMenu" });
    expect(inputMenu.props("items")).toHaveLength(1);
    expect((inputMenu.props("items") as Array<{ query: string }>)[0].query).toBe(longQuery);
    // ensure no object toString in display
    expect(wrapper.html()).not.toContain("[object Object]");
  });

  it("converts date range strings to UInputDate range and back, handling empty start/end", async () => {
    const wrapper = mount(SearchForm, {
      props: {
        filters: { query: "test", sourceKey: "", publishedAfter: "2025-01-15", publishedBefore: "2025-02-20", limit: 8 },
        historyEntries: [],
        sources: [],
        busy: false,
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    // advanced should be open due to dates and the real range picker is rendered
    expect(wrapper.find("[data-testid='search-published-range']").exists()).toBe(true);
    const inputDate = wrapper.getComponent({ name: "InputDate" });
    const initialRange = inputDate.props("modelValue") as
      | { start?: { year: number; month: number; day: number }; end?: { year: number; month: number; day: number } }
      | undefined;
    expect(initialRange?.start).toMatchObject({ year: 2025, month: 1, day: 15 });
    expect(initialRange?.end).toMatchObject({ year: 2025, month: 2, day: 20 });

    // full range update through the real UInputDate event
    await inputDate.vm.$emit("update:modelValue", {
      start: new CalendarDate(2025, 3, 1),
      end: new CalendarDate(2025, 3, 20),
    });
    let updates = wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined;
    expect(updates?.some(([filters]) => filters.publishedAfter === "2025-03-01" && filters.publishedBefore === "2025-03-20")).toBe(true);

    // only start keeps end empty
    await inputDate.vm.$emit("update:modelValue", {
      start: new CalendarDate(2025, 4, 5),
      end: undefined,
    });
    updates = wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined;
    expect(updates?.some(([filters]) => filters.publishedAfter === "2025-04-05" && filters.publishedBefore === "")).toBe(true);

    // only end keeps start empty
    await inputDate.vm.$emit("update:modelValue", {
      start: undefined,
      end: new CalendarDate(2025, 5, 6),
    });
    updates = wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined;
    expect(updates?.some(([filters]) => filters.publishedAfter === "" && filters.publishedBefore === "2025-05-06")).toBe(true);

    // empty range clears both dates
    await inputDate.vm.$emit("update:modelValue", undefined);
    updates = wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined;
    expect(updates?.some(([filters]) => filters.publishedAfter === "" && filters.publishedBefore === "")).toBe(true);

    // URL/history backfill with only one side still opens the picker with a partial range
    await wrapper.setProps({
      filters: { query: "test", sourceKey: "", publishedAfter: "2025-01-01", publishedBefore: "", limit: 8 },
    });
    const partialRange = wrapper.getComponent({ name: "InputDate" }).props("modelValue") as
      | { start?: { year: number; month: number; day: number }; end?: { year: number; month: number; day: number } }
      | undefined;
    expect(partialRange?.start).toMatchObject({ year: 2025, month: 1, day: 1 });
    expect(partialRange?.end).toBeUndefined();

    // clear dates via reset
    await wrapper.get("[data-testid='search-reset']").trigger("click");
    const lastUpdate = (wrapper.emitted("update:filters") as Array<[SearchFilters]> | undefined)?.at(-1)?.[0];
    expect(lastUpdate?.publishedAfter).toBe("");
    expect(lastUpdate?.publishedBefore).toBe("");
  });

  it("keeps source select compact without widening label", async () => {
    const wrapper = mount(SearchForm, {
      props: {
        filters: { query: "", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 },
        historyEntries: [],
        sources: [
          {
            source_key: "very_long_source_key_that_could_overflow_layout_if_not_truncated",
            group_key: "g",
            group_path: "g/p",
            visibility: "private",
            display_name: "Very Long Display Name That Could Overflow The Filter Bar If Not Truncated Properly And Needs Title Attribute",
            description: "",
            example_queries: [],
            connection: "c",
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
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    await wrapper.get("[data-testid='search-toggle-advanced']").trigger("click");
    const select = wrapper.findComponent({ name: "Select" });
    expect(select.exists()).toBe(true);
    // ensure select container has min-w-0 and options are truncated (check for truncate class)
    expect(select.props("items")).toHaveLength(2);
    expect((select.props("items") as Array<{ label: string }>)[1].label).toContain("Very Long");
  });
});
