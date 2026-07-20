import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { setAuthenticatedUser } from "../test-utils/auth";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import AppRouteBreadcrumbs from "./AppRouteBreadcrumbs.vue";

describe("AppRouteBreadcrumbs", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setAuthenticatedUser();
  });

  it("hides the single browser breadcrumb on the groups root", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/groups", name: "groups", component: { template: "<div />" } }],
    });
    router.push("/groups");
    await router.isReady();

    const wrapper = mount(AppRouteBreadcrumbs, {
      global: { plugins: [testNuxtUiPlugin, router, createAppI18n("zh-CN")] },
    });

    expect(wrapper.find("nav").exists()).toBe(false);
    expect(wrapper.find("#app-route-actions").exists()).toBe(true);
  });

  it("renders browser and group path on a group overview", async () => {
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
        plugins: [testNuxtUiPlugin, router, createAppI18n("zh-CN")],
      },
    });

    expect(wrapper.find("nav").exists()).toBe(true);
    expect(wrapper.text()).toContain("浏览器");
    expect(wrapper.text()).toContain("stock");
    expect(wrapper.find("#app-route-actions").exists()).toBe(true);
  });

  it("keeps the processing queue route out of unrelated breadcrumbs", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
    });
    router.push("/processing-queue");
    await router.isReady();

    const wrapper = mount(AppRouteBreadcrumbs, {
      global: { plugins: [testNuxtUiPlugin, router, createAppI18n("en")] },
    });

    expect(wrapper.find("nav").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Search");
  });

  it("renders navigable group path before the current section", async () => {
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
        plugins: [testNuxtUiPlugin, router, createAppI18n("en")],
      },
    });

    await flushPromises();

    expect(wrapper.text()).toContain("Browser");
    expect(wrapper.text()).toContain("stock");
    expect(wrapper.text()).toContain("alpha");
    expect(wrapper.text()).toContain("Settings");
    expect(wrapper.text()).not.toContain("Search");

    const stockLink = wrapper.findAll("a").find((link) => link.text() === "stock");
    expect(stockLink).toBeDefined();
    await stockLink!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock");

    const browserLink = wrapper.findAll("a").find((link) => link.text() === "Browser");
    expect(browserLink).toBeDefined();
    await browserLink!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.name).toBe("groups");
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
        plugins: [testNuxtUiPlugin, router, createAppI18n("zh-CN")],
      },
    });

    expect(wrapper.text()).toContain("设置");
    expect(wrapper.text()).toContain("访问令牌");
    expect(wrapper.text()).not.toContain("搜索");
  });
});
