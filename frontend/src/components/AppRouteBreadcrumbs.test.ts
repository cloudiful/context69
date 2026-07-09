import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { setAuthenticatedUser } from "../test-utils/auth";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import {
  setWorkspaceNavigationGroup,
  setWorkspaceNavigationProject,
} from "../composables/use-workspace-navigation-context";
import AppRouteBreadcrumbs from "./AppRouteBreadcrumbs.vue";

describe("AppRouteBreadcrumbs", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setAuthenticatedUser();
    setWorkspaceNavigationGroup("", "");
    setWorkspaceNavigationProject("", "", "");
  });

  it("renders group and project hierarchy and navigates back to the group", async () => {
    setWorkspaceNavigationGroup("stock", "Stock Team");
    setWorkspaceNavigationProject("stock", "alpha", "Alpha Project");

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: { template: "<div />" } },
        { path: "/groups", name: "groups", component: { template: "<div />" } },
        { path: "/groups/:groupKey/overview", name: "group-overview", component: { template: "<div />" } },
        { path: "/groups/:groupKey/projects/:projectKey/overview", name: "project-overview", component: { template: "<div />" } },
        { path: "/groups/:groupKey/projects/:projectKey/settings", name: "project-settings", component: { template: "<div />" } },
      ],
    });

    router.push("/groups/stock/projects/alpha/settings");
    await router.isReady();

    const wrapper = mount(AppRouteBreadcrumbs, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    await flushPromises();

    expect(wrapper.text()).toContain("Search");
    expect(wrapper.text()).toContain("Groups");
    expect(wrapper.text()).toContain("Stock Team");
    expect(wrapper.text()).toContain("Alpha Project");
    expect(wrapper.text()).toContain("Settings");

    const groupButton = wrapper.findAll("button").find((button) => button.text() === "Stock Team");
    expect(groupButton).toBeDefined();

    await groupButton!.trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("group-overview");
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
