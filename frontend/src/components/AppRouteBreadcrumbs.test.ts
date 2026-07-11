import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { setAuthenticatedUser } from "../test-utils/auth";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import AppRouteBreadcrumbs from "./AppRouteBreadcrumbs.vue";

describe("AppRouteBreadcrumbs", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setAuthenticatedUser();
  });

  it("hides a breadcrumb with only one node", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div />" } },
      ],
    });
    router.push("/groups/stock");
    await router.isReady();

    const wrapper = mount(AppRouteBreadcrumbs, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("zh-CN")],
      },
    });

    expect(wrapper.find(".p-breadcrumb").exists()).toBe(false);
    expect(wrapper.find("#app-route-actions").exists()).toBe(true);
  });

  it("renders browser and the current group section only", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: { template: "<div />" } },
        { path: "/groups", name: "groups", component: { template: "<div />" } },
        { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div />" } },
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

    expect(wrapper.text()).toContain("Browser");
    expect(wrapper.text()).toContain("Settings");
    expect(wrapper.text()).not.toContain("Search");
    expect(wrapper.text()).not.toContain("stock");
    expect(wrapper.text()).not.toContain("Alpha Group");

    const browserButton = wrapper.findAll("button").find((button) => button.text() === "Browser");
    expect(browserButton).toBeDefined();

    await browserButton!.trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("group-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock%2Falpha");
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

    expect(wrapper.text()).toContain("设置");
    expect(wrapper.text()).toContain("访问令牌");
    expect(wrapper.text()).not.toContain("搜索");
  });
});
