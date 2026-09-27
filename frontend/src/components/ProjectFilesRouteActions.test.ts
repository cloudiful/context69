import { mount } from "@vue/test-utils";
import type { DropdownMenuItem } from "@nuxt/ui";
import { nextTick } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import ProjectFilesRouteActions from "./ProjectFilesRouteActions.vue";

const menuItems: DropdownMenuItem[] = [
  { label: "New text file", icon: "i-lucide-file-plus" },
  { label: "Upload files", icon: "i-lucide-upload" },
];

type ActionsOverrides = Partial<{
  deleteSourceAfterProcessing: boolean;
  query: string;
  retryAllBusy: boolean;
  retryAllFailedCount: number;
  scopedQuery: string;
  scopedSearchLoading: boolean;
  scopedSearchPlaceholder: string;
  uploadBusy: boolean;
}>;

function mountActions(overrides: ActionsOverrides = {}) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/", component: { template: "<div />" } }],
  });

  return mount(ProjectFilesRouteActions, {
    props: {
      createMenuItems: menuItems,
      deleteSourceAfterProcessing: false,
      query: "",
      retryAllBusy: false,
      retryAllFailedCount: 0,
      scopedQuery: "",
      scopedSearchLoading: false,
      scopedSearchPlaceholder: "Search content in reports",
      uploadBusy: false,
      ...overrides,
    },
    global: {
      plugins: [testNuxtUiPlugin, router, createTestI18n()],
      stubs: { teleport: true },
    },
  });
}

describe("ProjectFilesRouteActions", () => {
  it("keeps the route-actions testid and a11y surface", () => {
    const i18n = createTestI18n();
    const t = i18n.global.t;
    const wrapper = mountActions();

    const scopedInput = wrapper.get('[data-testid="scoped-content-search-input"]');
    expect(scopedInput.attributes("placeholder")).toBe("Search content in reports");
    expect(scopedInput.attributes("aria-label")).toBe("Search content in reports");
    expect(scopedInput.attributes("title")).toBe("Search content in reports");

    expect(wrapper.get('[data-testid="scoped-content-search-trigger"]').text()).toContain(
      t("search.scoped.run"),
    );

    const retryAll = wrapper.get('[data-testid="retry-all-failed"]');
    expect(retryAll.attributes("title")).toBe(t("library.retryAllFailed"));

    const retryAllCompact = wrapper.get('[data-testid="retry-all-failed-compact"]');
    expect(retryAllCompact.attributes("aria-label")).toBe("Retry all failed");

    expect(
      wrapper.findAll("button").some((button) => button.text().includes(t("common.new"))),
    ).toBe(true);
    expect(wrapper.find('[aria-label="New"]').exists()).toBe(true);

    expect(wrapper.find('[aria-label="Release source after processing"]').exists()).toBe(true);

    expect(
      wrapper.findAll("button").some((button) => button.text().includes(t("common.upload"))),
    ).toBe(true);
    expect(wrapper.find('[aria-label="Upload"]').exists()).toBe(true);

    wrapper.unmount();
  });

  it("emits query and scoped-query updates from the toolbar inputs", async () => {
    const wrapper = mountActions();
    const inputs = wrapper.findAllComponents({ name: "Input" });

    expect(inputs).toHaveLength(2);

    await inputs[0].setValue("alpha");
    await inputs[1].setValue("beta");

    expect(wrapper.emitted("update:query")).toEqual([["alpha"]]);
    expect(wrapper.emitted("update:scopedQuery")).toEqual([["beta"]]);

    wrapper.unmount();
  });

  it("runs the scoped search from the Enter key and the trigger button", async () => {
    const wrapper = mountActions();

    await wrapper.get('[data-testid="scoped-content-search-input"]').trigger("keydown.enter");
    expect(wrapper.emitted("runScopedSearch")).toHaveLength(1);

    await wrapper.get('[data-testid="scoped-content-search-trigger"]').trigger("click");
    expect(wrapper.emitted("runScopedSearch")).toHaveLength(2);

    wrapper.unmount();
  });

  it("emits retryAllFailed from both retry buttons and tracks the busy/count state", async () => {
    const idle = mountActions({ retryAllFailedCount: 0 });

    expect(idle.get('[data-testid="retry-all-failed"]').attributes("disabled")).toBeDefined();
    expect(idle.get('[data-testid="retry-all-failed-compact"]').attributes("disabled")).toBeDefined();

    await idle.get('[data-testid="retry-all-failed"]').trigger("click");
    expect(idle.emitted("retryAllFailed")).toBeUndefined();
    idle.unmount();

    const ready = mountActions({ retryAllFailedCount: 3 });

    expect(ready.get('[data-testid="retry-all-failed"]').attributes("disabled")).toBeUndefined();
    await ready.get('[data-testid="retry-all-failed"]').trigger("click");
    await ready.get('[data-testid="retry-all-failed-compact"]').trigger("click");

    expect(ready.emitted("retryAllFailed")).toHaveLength(2);
    ready.unmount();

    const busy = mountActions({ retryAllBusy: true, retryAllFailedCount: 3 });

    expect(busy.get('[data-testid="retry-all-failed"]').attributes("disabled")).toBeDefined();
    expect(busy.get('[data-testid="retry-all-failed-compact"]').attributes("disabled")).toBeDefined();
    busy.unmount();
  });

  it("emits upload from the desktop and compact upload buttons", async () => {
    const i18n = createTestI18n();
    const wrapper = mountActions();

    const uploadDesktop = wrapper
      .findAll("button")
      .find((button) => button.text().includes(i18n.global.t("common.upload")));

    expect(uploadDesktop).toBeDefined();
    await uploadDesktop!.trigger("click");

    await wrapper.get('[aria-label="Upload"]').trigger("click");

    expect(wrapper.emitted("upload")).toHaveLength(2);

    wrapper.unmount();
  });

  it("round-trips the release-source checkbox through the boolean emit", async () => {
    const wrapper = mountActions({ deleteSourceAfterProcessing: true });
    const checkbox = wrapper.getComponent({ name: "Checkbox" });

    expect(checkbox.props("modelValue")).toBe(true);

    checkbox.vm.$emit("update:modelValue", true);
    checkbox.vm.$emit("update:modelValue", false);
    await nextTick();

    expect(wrapper.emitted("update:deleteSourceAfterProcessing")).toEqual([[true], [false]]);

    wrapper.unmount();
  });
});
