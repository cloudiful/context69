import { flushPromises, mount } from "@vue/test-utils";
import InputNumber from "primevue/inputnumber";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";

const {
  listSources,
  listSourceConnections,
  syncSource,
  createSource,
  updateSource,
  deleteSource,
} = vi.hoisted(() => ({
  listSources: vi.fn(),
  listSourceConnections: vi.fn(),
  syncSource: vi.fn(),
  createSource: vi.fn(),
  updateSource: vi.fn(),
  deleteSource: vi.fn(),
}));

vi.mock("../services/api", () => ({
  apiClient: {
    listSources,
    listSourceConnections,
    syncSource,
    createSource,
    updateSource,
    deleteSource,
  },
}));

vi.mock("../components/AppMonacoEditor.vue", () => ({
  default: {
    props: ["modelValue", "inputId"],
    emits: ["update:modelValue"],
    template: `
      <textarea
        :id="inputId"
        :value="modelValue"
        @input="$emit('update:modelValue', $event.target.value)"
      />
    `,
  },
}));

vi.mock("primevue/useconfirm", () => ({
  useConfirm: () => ({
    require: (options: { accept?: () => void }) => options.accept?.(),
    close: vi.fn(),
  }),
}));

import SourcesView from "./SourcesView.vue";

const baseSource = {
  source_key: "gov_documents",
  group_key: "personal-admin",
  project_key: "default",
  visibility: "private",
  display_name: "国务院/部委政策公文",
  description: "覆盖国务院及部委正式政策公文。",
  example_queries: ["新能源汽车 购置税 政策", "国务院 关于 数据要素 的意见"],
  connection: "gov-info",
  has_database_url: true,
  origin_status: "connected" as const,
  origin_message: null,
  sync_strategy: "cursor",
  connector_type: "postgres_sql",
  base_query: "SELECT 1",
  batch_size: 200,
  last_cursor_updated_at: null,
  last_cursor_external_id: null,
  last_success_at: null,
};

describe("SourcesView", () => {
  beforeEach(() => {
    listSources.mockReset();
    listSourceConnections.mockReset();
    syncSource.mockReset();
    createSource.mockReset();
    updateSource.mockReset();
    deleteSource.mockReset();
    vi.restoreAllMocks();
  });

  it("loads sources and syncs one row independently", async () => {
    listSources
      .mockResolvedValueOnce([baseSource])
      .mockResolvedValueOnce([
        {
          ...baseSource,
          last_cursor_updated_at: "2025-01-01T00:00:00Z",
          last_cursor_external_id: "a1",
          last_success_at: "2025-01-01T00:00:00Z",
        },
      ]);
    listSourceConnections.mockResolvedValue([{ name: "gov-info", has_database_url: true, origin_status: "connected", origin_message: null }]);
    syncSource.mockResolvedValue({
      records_seen: 10,
      records_changed: 2,
      chunks_upserted: 6,
    });

    const wrapper = mount(SourcesView, {
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n()],
      },
    });
    await flushPromises();

    expect(wrapper.find(".source-card-list").exists()).toBe(true);
    expect(wrapper.find(".tool-card").text()).toContain("国务院/部委政策公文");

    const syncButton = wrapper.findAll("button").find((button) => button.text() === "Sync");
    expect(syncButton).toBeTruthy();
    await syncButton!.trigger("click");
    await flushPromises();

    expect(syncSource).toHaveBeenCalledWith("gov_documents");
    expect(listSources).toHaveBeenCalledTimes(2);
  });

  it("creates, updates, and deletes a source", async () => {
    listSources
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([baseSource])
      .mockResolvedValueOnce([{ ...baseSource, batch_size: 500 }])
      .mockResolvedValueOnce([]);
    listSourceConnections.mockResolvedValue([{ name: "gov-info", has_database_url: true, origin_status: "connected", origin_message: null }]);
    createSource.mockResolvedValue(baseSource);
    updateSource.mockResolvedValue({ ...baseSource, batch_size: 500 });
    deleteSource.mockResolvedValue(undefined);

    const wrapper = mount(SourcesView, {
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n()],
      },
    });
    await flushPromises();

    await wrapper.get("#source-key").setValue("gov_documents");
    await wrapper.get("#source-display-name").setValue("国务院/部委政策公文");
    await wrapper.get("#source-description").setValue("覆盖国务院及部委正式政策公文。");
    await wrapper.get("#source-example-queries").setValue("新能源汽车 购置税 政策\n国务院 关于 数据要素 的意见");
    await wrapper.get("#source-base-query").setValue("SELECT 1");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(createSource).toHaveBeenCalledWith(
      expect.objectContaining({
        source_key: "gov_documents",
        display_name: "国务院/部委政策公文",
        description: "覆盖国务院及部委正式政策公文。",
        example_queries: ["新能源汽车 购置税 政策", "国务院 关于 数据要素 的意见"],
        connection: "gov-info",
        connector_type: "postgres_sql",
      }),
    );

    const editButton = wrapper.findAll("button").find((button) => button.text() === "Edit");
    expect(editButton).toBeTruthy();
    await editButton!.trigger("click");
    expect((wrapper.get("#source-base-query").element as HTMLTextAreaElement).value).toBe("SELECT 1");
    const inputNumbers = wrapper.findAllComponents(InputNumber);
    expect(inputNumbers.length).toBeGreaterThan(0);
    await inputNumbers[0].vm.$emit("update:modelValue", 500);
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(updateSource).toHaveBeenCalledWith(
      "gov_documents",
      expect.objectContaining({
        batch_size: 500,
      }),
    );

    const deleteButton = wrapper.findAll("button").find((button) => button.text() === "Delete");
    expect(deleteButton).toBeTruthy();
    await deleteButton!.trigger("click");
    await flushPromises();

    expect(deleteSource).toHaveBeenCalledWith("gov_documents");
  });
});
