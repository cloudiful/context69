import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import LibraryResourceTable from "./LibraryResourceTable.vue";

describe("LibraryResourceTable", () => {
  it("shows a retry action instead of the empty-folder state after a load failure", async () => {
    const wrapper = mount(LibraryResourceTable, {
      props: {
        createFolderBusy: false,
        entries: [],
        error: "Failed to load the file library",
        expandedKeys: {},
        loading: false,
        resourceSearchQuery: "",
        selectedFolderReady: false,
        selection: null,
        tableContextSelection: null,
        uploadBusy: false,
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
});
