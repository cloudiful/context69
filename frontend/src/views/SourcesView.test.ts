import { flushPromises, mount } from "@vue/test-utils";
import InputNumber from "@nuxt/ui/components/InputNumber.vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { apiClient, type SourceStatus } from "../services/api";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import SourcesView from "./SourcesView.vue";

const listSources = vi.spyOn(apiClient, "listSources");
const listSourceConnections = vi.spyOn(apiClient, "listSourceConnections");
const syncSource = vi.spyOn(apiClient, "syncSource");
const createSource = vi.spyOn(apiClient, "createSource");
const updateSource = vi.spyOn(apiClient, "updateSource");
const deleteSource = vi.spyOn(apiClient, "deleteSource");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");

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

const appMonacoEditorStub = {
  props: ["modelValue", "inputId"],
  emits: ["update:modelValue"],
  template: `
    <textarea
      :id="inputId"
      :value="modelValue"
      @input="$emit('update:modelValue', $event.target.value)"
    />
  `,
};

const baseSource: SourceStatus = {
  source_key: "gov_documents",
  group_key: "personal-admin",
  group_path: "personal-admin/default",
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
    useOverlay.mockReset();
    useOverlay.mockReturnValue({
      create: () => ({ open: async () => true }),
    } as never);
    listSources.mockReset();
    listSourceConnections.mockReset();
    syncSource.mockReset();
    createSource.mockReset();
    updateSource.mockReset();
    deleteSource.mockReset();
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
        plugins: [testNuxtUiPlugin, createTestI18n()],
        stubs: { AppMonacoEditor: appMonacoEditorStub },
      },
    });
    await flushPromises();

    expect(wrapper.findComponent({ name: "Table" }).exists()).toBe(true);
    expect(wrapper.text()).toContain("国务院/部委政策公文");

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
        plugins: [testNuxtUiPlugin, createTestI18n()],
        stubs: { AppMonacoEditor: appMonacoEditorStub },
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
    const inputNumbers = wrapper.findAllComponents({ name: "InputNumber" });
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
    const confirmButton = [...document.body.querySelectorAll("button")].find((button) => (
      button !== deleteButton!.element && button.textContent?.trim() === "Delete"
    ));
    if (confirmButton) {
      confirmButton.click();
    }
    await flushPromises();

    expect(deleteSource).toHaveBeenCalledWith("gov_documents");
  });
});
