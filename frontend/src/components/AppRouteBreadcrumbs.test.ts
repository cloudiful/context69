import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { setAuthenticatedUser } from "../test-utils/auth";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { setWorkspaceNavigationGroup } from "../composables/use-workspace-navigation-context";
import AppRouteBreadcrumbs from "./AppRouteBreadcrumbs.vue";

describe("AppRouteBreadcrumbs", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setAuthenticatedUser();
    setWorkspaceNavigationGroup("", "");
  });

  it("renders group hierarchy and navigates back to the group", async () => {
    setWorkspaceNavigationGroup("stock/alpha", "Alpha Group");

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: { template: "<div />" } },
        { path: "/groups", name: "groups", component: { template: "<div />" } },
        { path: "/groups/:groupPath/overview", name: "group-overview", component: { template: "<div />" } },
        { path: "/groups/:groupPath/settings", name: "group-settings", component: { template: "<div />" } },
      ],
    });

    router.push("/groups/stock%2Falpha/settings");
    await router.isReady();

    const wrapper = mount(AppRouteBreadcrumbs, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    await flushPromises();

    expect(wrapper.text()).toContain("Search");
    expect(wrapper.text()).toContain("Browser");
    expect(wrapper.text()).toContain("stock");
    expect(wrapper.text()).toContain("Alpha Group");
    expect(wrapper.text()).toContain("Settings");

    const groupButton = wrapper.findAll("button").find((button) => button.text() === "Alpha Group");
    expect(groupButton).toBeDefined();

    await groupButton!.trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("group-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock%2Falpha/overview");

    const parentGroupButton = wrapper.findAll("button").find((button) => button.text() === "stock");
    expect(parentGroupButton).toBeDefined();

    await parentGroupButton!.trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/overview");
  });

  it("renders settings breadcrumbs for nested settings routes", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: { template: "<div />" } },
        { path: "/settings/appearance", name: "settings-appearance", component: { template: "<div />" } },
        { path: "/settings/access-tokens", name: "settings-access-tokens", component: { template: "<div />" } },
      ],
    });

    router.push("/settings/access-tokens");
    await router.isReady();

    const wrapper = mount(AppRouteBreadcrumbs, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("zh-CN")],
      },
    });

    expect(wrapper.text()).toContain("搜索");
    expect(wrapper.text()).toContain("设置");
    expect(wrapper.text()).toContain("访问令牌");
  });
});
