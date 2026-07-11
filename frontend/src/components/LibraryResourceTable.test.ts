import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { installMockStorage } from "../test-utils/storage";
import LibraryResourceTable from "./LibraryResourceTable.vue";
import type { ExplorerEntry } from "../types/library";

describe("LibraryResourceTable", () => {
  const baseProps = {
    createFolderBusy: false,
    entries: [],
    expandedKeys: {},
    loading: false,
    resourceSearchQuery: "",
    selectedFolderReady: true,
    selection: null,
    tableContextSelection: null,
    uploadBusy: false,
  };

  it("shows a retry action instead of the empty-folder state after a load failure", async () => {
    const wrapper = mount(LibraryResourceTable, {
      props: {
        ...baseProps,
        error: "Failed to load the file library",
        selectedFolderReady: false,
      },
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n()],
      },
    });

    expect(wrapper.text()).toContain("Failed to load the file library");
    expect(wrapper.text()).not.toContain("Upload a file or create subfolders");

    await wrapper.get("button").trigger("click");
    expect(wrapper.emitted("retry")).toHaveLength(1);
  });

  it("forwards lazy pagination and sortable column events", async () => {
    const wrapper = mount(LibraryResourceTable, {
      props: {
        ...baseProps,
        paginated: true,
        totalRecords: 120,
      },
      global: { plugins: [testPrimeVuePlugin, createTestI18n()] },
    });
    const table = wrapper.findComponent({ name: "DataTable" });

    table.vm.$emit("page", { first: 50, rows: 50 });
    table.vm.$emit("sort", { sortField: "updated_at", sortOrder: -1 });
    table.vm.$emit("sort", { sortField: "updated_at", sortOrder: 1 });
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted("page")?.[0]).toEqual([{ first: 50, rows: 50 }]);
    expect(wrapper.emitted("sort")?.[0]).toEqual([{ sortField: "updated_at", sortOrder: -1 }]);
    expect(wrapper.emitted("sort")?.[1]).toEqual([{ sortField: "updated_at", sortOrder: 1 }]);
  });

  it("forwards desktop and mobile status filters", async () => {
    const wrapper = mount(LibraryResourceTable, {
      props: {
        ...baseProps,
        paginated: true,
      },
      global: { plugins: [testPrimeVuePlugin, createTestI18n()] },
    });

    wrapper.findComponent({ name: "DataTable" }).vm.$emit("filter", {
      filters: {
        status: {
          operator: "and",
          constraints: [{ value: "failed", matchMode: "equals" }],
        },
      },
    });
    const mobileStatusSelect = wrapper
      .findAllComponents({ name: "Select" })
      .find((select) => select.props("ariaLabel") === "Filter by status");
    mobileStatusSelect?.vm.$emit("update:modelValue", "running");
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted("status-filter")?.[0]).toEqual(["failed"]);
    expect(wrapper.emitted("status-filter")?.[1]).toEqual(["running"]);
  });

  it("persists column widths without restoring backend sort state", async () => {
    const storage = installMockStorage();
    const wrapper = mount(LibraryResourceTable, {
      props: { ...baseProps, compact: true, paginated: true },
      global: { plugins: [testPrimeVuePlugin, createTestI18n()] },
    });

    wrapper.findComponent({ name: "DataTable" }).vm.$emit("state-save", {
      columnWidths: "420,120,160,120,220",
      first: 50,
      rows: 50,
      sortField: "updated_at",
      sortOrder: 1,
      tableWidth: "1040px",
    });

    expect(JSON.parse(storage.getItem("context69:table:group-library:v5") ?? "{}")).toEqual({
      columnWidths: "420,120,160,120,220",
      tableWidth: "1040px",
    });
  });

  it("offers retry only for failed files", async () => {
    const failedFile = {
      key: "file:failed",
      kind: "file",
      id: "failed",
      depth: 0,
      name: "failed.txt",
      parentFolderId: null,
      path: "/",
      updatedAt: "2026-07-11T12:00:00Z",
      mediaType: "text/plain",
      sizeBytes: 12,
      ingestStatus: "failed",
      errorMessage: "embedding timeout",
      isSourceConfigFile: false,
      isSourceRecordFile: false,
      file: {
        file_id: "failed",
        group_key: "group",
        group_path: "group",
        visibility: "private",
        folder_id: null,
        filename: "failed.txt",
        media_type: "text/plain",
        size_bytes: 12,
        ingest_status: "failed",
        error_message: "embedding timeout",
        created_at: "2026-07-11T12:00:00Z",
        updated_at: "2026-07-11T12:00:00Z",
        ingested_at: null,
      },
    } satisfies ExplorerEntry;
    const wrapper = mount(LibraryResourceTable, {
      props: { ...baseProps, entries: [failedFile] },
      global: { plugins: [testPrimeVuePlugin, createTestI18n()] },
    });

    const retry = wrapper.get('button[title="Retry"]');
    await retry.trigger("click");

    expect(wrapper.emitted("retry-entry")?.[0]).toEqual([failedFile]);
  });
});
