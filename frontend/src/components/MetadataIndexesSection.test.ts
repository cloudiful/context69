import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type MetadataIndexResponse, type Pagination } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import MetadataIndexesSection from "./MetadataIndexesSection.vue";

const listMetadataIndexes = vi.spyOn(apiClient, "listMetadataIndexes");
const retryMetadataIndex = vi.spyOn(apiClient, "retryMetadataIndex");
const deleteMetadataIndex = vi.spyOn(apiClient, "deleteMetadataIndex");

const pagination: Pagination = { page: 1, page_size: 50, total: 1, total_pages: 1 };

function indexRow(status: MetadataIndexResponse["status"]): MetadataIndexResponse {
  return {
    index_id: "idx-1",
    group_path: "stock",
    source_key: "sales",
    path: "meta.price",
    data_type: "keyword",
    value_kind: "scalar",
    sortable: false,
    status,
    processed_documents: 0,
    total_documents: 10,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    error_message: null,
  };
}

describe("MetadataIndexesSection", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    listMetadataIndexes.mockReset();
    retryMetadataIndex.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  async function mountSection() {
    const wrapper = mount(MetadataIndexesSection, {
      props: { groupPath: "stock", canManage: true },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });
    await wrapper.get("input").setValue("sales");
    await wrapper.get("input").trigger("keyup.enter");
    await flushPromises();
    return wrapper;
  }

  it("polls after retry until the index settles", async () => {
    listMetadataIndexes
      .mockResolvedValueOnce({ items: [indexRow("failed")], pagination } as never)
      .mockResolvedValueOnce({ items: [indexRow("building")], pagination } as never)
      .mockResolvedValueOnce({ items: [indexRow("ready")], pagination } as never);
    retryMetadataIndex.mockResolvedValue(indexRow("building") as never);

    const wrapper = await mountSection();
    expect(listMetadataIndexes).toHaveBeenCalledTimes(1);

    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry"));
    expect(retryButton).toBeTruthy();
    await retryButton!.trigger("click");
    await flushPromises();
    expect(listMetadataIndexes).toHaveBeenCalledTimes(2);

    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();
    expect(listMetadataIndexes).toHaveBeenCalledTimes(3);
    expect(wrapper.text()).toContain("ready");
    wrapper.unmount();
  });

  it("stops polling once the index is ready", async () => {
    listMetadataIndexes
      .mockResolvedValueOnce({ items: [indexRow("failed")], pagination } as never)
      .mockResolvedValueOnce({ items: [indexRow("ready")], pagination } as never);
    retryMetadataIndex.mockResolvedValue(indexRow("ready") as never);

    const wrapper = await mountSection();
    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry"));
    await retryButton!.trigger("click");
    await flushPromises();
    expect(listMetadataIndexes).toHaveBeenCalledTimes(2);

    await vi.advanceTimersByTimeAsync(6000);
    await flushPromises();
    expect(listMetadataIndexes).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("polls after delete until the index disappears", async () => {
    deleteMetadataIndex.mockReset();
    deleteMetadataIndex.mockResolvedValue(undefined as never);
    listMetadataIndexes
      .mockResolvedValueOnce({ items: [indexRow("ready")], pagination } as never)
      .mockResolvedValueOnce({ items: [indexRow("deleting")], pagination } as never)
      .mockResolvedValueOnce({ items: [], pagination: { ...pagination, total: 0 } } as never);

    const wrapper = await mountSection();
    const deleteButton = wrapper.findAll("button").find((button) => button.text().includes("Delete"));
    expect(deleteButton).toBeTruthy();
    await deleteButton!.trigger("click");
    await flushPromises();
    expect(deleteMetadataIndex).toHaveBeenCalledWith("stock", "idx-1");
    expect(listMetadataIndexes).toHaveBeenCalledTimes(2);

    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();
    expect(listMetadataIndexes).toHaveBeenCalledTimes(3);
    wrapper.unmount();
  });
});
