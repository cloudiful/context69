import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type SearchHit, type SearchResponse } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { useScopedContentSearch } from "./use-scoped-content-search";

const search = vi.spyOn(apiClient, "search");

function hit(patch: Partial<SearchHit>): SearchHit {
  return {
    chunk_id: "c1",
    document_id: 1,
    source_key: "src",
    external_id: "e1",
    group_key: "g",
    group_path: "stock/alpha",
    visibility: "private",
    title: "Title",
    summary: "",
    source_uri: "",
    published_at: null,
    chunk_index: 0,
    chunk_text: "text",
    score: 0.5,
    metadata_json: {},
    is_library_file: true,
    library_path: "/docs/sub/file.md",
    ...patch,
  };
}

function response(items: SearchHit[]): SearchResponse {
  return { query: "t", items, pagination: { page: 1, page_size: 20, total: items.length, total_pages: 1 } };
}

describe("useScopedContentSearch", () => {
  beforeEach(() => {
    search.mockReset();
  });

  it("narrows to the whole group via group_path and filters by folder path prefix", async () => {
    search.mockResolvedValue(response([
      hit({ chunk_id: "inside", library_path: "/docs/sub/file.md" }),
      hit({ chunk_id: "outside", library_path: "/other/file.md" }),
      hit({ chunk_id: "nonlib", is_library_file: false, library_path: null }),
    ]));

    let state!: ReturnType<typeof useScopedContentSearch>;
    const folderPath = ref("/docs/sub");
    const wrapper = mount(defineComponent({
      setup() {
        state = useScopedContentSearch({ groupPath: "stock/alpha", folderPath, t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    state.query.value = "policy";
    await state.run();

    expect(search).toHaveBeenCalledWith(
      expect.objectContaining({ query: "policy", group_path: "stock/alpha", page: 1, limit: 20 }),
      expect.any(Object),
    );
    expect(state.results.value.map((item) => item.chunk_id)).toEqual(["inside"]);
    expect(state.modalVisible.value).toBe(true);
    wrapper.unmount();
  });

  it("does not open the modal when nothing matches in the folder", async () => {
    search.mockResolvedValue(response([hit({ chunk_id: "outside", library_path: "/other/file.md" })]));

    let state!: ReturnType<typeof useScopedContentSearch>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useScopedContentSearch({ groupPath: "stock/alpha", folderPath: "/docs", t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    state.query.value = "policy";
    await state.run();

    expect(state.results.value).toHaveLength(0);
    expect(state.modalVisible.value).toBe(false);
    wrapper.unmount();
  });

  it("does not search when the query is empty", async () => {
    let state!: ReturnType<typeof useScopedContentSearch>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useScopedContentSearch({ groupPath: "stock/alpha", folderPath: "/docs", t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });

    state.query.value = "   ";
    await state.run();

    expect(search).not.toHaveBeenCalled();
    expect(state.modalVisible.value).toBe(false);
    wrapper.unmount();
  });
});
