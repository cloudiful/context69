import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { useUiPreferences } from "../composables/use-ui-preferences";
import AppSidebarContent from "./AppSidebarContent.vue";

const sidebarRoutes = [
  { path: "/login", component: { template: "<div />" } },
  { path: "/search", component: { template: "<div />" } },
  { path: "/groups", component: { template: "<div />" } },
  { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div />" } },
  { path: "/groups/:groupPath/members", name: "group-members", component: { template: "<div />" } },
  { path: "/groups/:groupPath/settings", name: "group-settings", component: { template: "<div />" } },
  { path: "/settings", component: { template: "<div />" } },
  { path: "/settings/:section", component: { template: "<div />" } },
];

async function routerAt(path: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: sidebarRoutes,
  });
  await router.push(path);
  await router.isReady();
  return router;
}

describe("AppSidebarContent", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setGuest();
  });

  it("renders business navigation with the current route highlighted", async () => {
    setAuthenticatedUser();

    const router = await routerAt("/search");

    const wrapper = mount(AppSidebarContent, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    expect(wrapper.text()).toContain("Search");
    expect(wrapper.text()).toContain("Settings");
    expect(wrapper.text()).toContain("Administrator");
    expect(wrapper.get('[data-testid="sidebar-user-login"]').text()).toBe("@admin");
    expect(wrapper.get('[data-testid="sidebar-user-badge"]').text()).toContain("Administrator");
    expect(wrapper.find('[data-nav-key="/search"]').attributes("data-active")).toBe("true");
    expect(wrapper.get('[aria-label="Log Out"]').attributes("aria-label")).toBe("Log Out");
  });

  it("renders settings secondary navigation when the settings section is active", async () => {
    setAuthenticatedUser();

    const router = await routerAt("/settings/access-tokens");

    const wrapper = mount(AppSidebarContent, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("zh-CN")],
      },
    });

    expect(wrapper.find('[data-nav-key="/settings"]').attributes("data-active")).toBe("true");
    expect(wrapper.get('[data-nav-child-key="/settings/access-tokens"]').attributes("data-active")).toBe("true");
    expect(wrapper.text()).toContain("访问令牌");
    expect(wrapper.text()).toContain("运行时");
  });

  it("renders group secondary navigation when a group route is active", async () => {
    setAuthenticatedUser();

    const router = await routerAt("/groups/stock/members");

    const wrapper = mount(AppSidebarContent, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    expect(wrapper.find('[data-nav-key="/groups"]').attributes("data-active")).toBe("true");
    expect(wrapper.get('[data-nav-key="/groups"]').attributes("href")).toBe("/groups/stock");
    expect(wrapper.get('[data-nav-child-key="/groups/stock/members"]').attributes("data-active")).toBe("true");
    expect(wrapper.text()).toContain("Members");
    expect(wrapper.text()).toContain("Settings");
    expect(wrapper.text()).not.toContain("Overview");
    expect(wrapper.find(".app-sidebar-subnav-heading").exists()).toBe(false);
  });

  it("hides business navigation when the session is not authenticated", async () => {
    const router = await routerAt("/login");

    const wrapper = mount(AppSidebarContent, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    await flushPromises();

    expect(wrapper.text()).not.toContain("Search");
    expect(wrapper.html()).not.toContain('aria-label="Log Out"');
  });

  it("hides user details but keeps logout accessible when collapsed", async () => {
    setAuthenticatedUser();

    const router = await routerAt("/search");

    const wrapper = mount(AppSidebarContent, {
      props: {
        collapsed: true,
      },
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    expect(wrapper.text()).not.toContain("Administrator");
    expect(wrapper.get('[aria-label="Log Out"]').attributes("aria-label")).toBe("Log Out");
  });

  it("expands the desktop sidebar when a collapsed top-level item has secondary navigation", async () => {
    setAuthenticatedUser();
    const preferences = useUiPreferences();
    preferences.state.sidebarCollapsed = true;

    const router = await routerAt("/settings/appearance");

    const wrapper = mount(AppSidebarContent, {
      props: {
        collapsed: true,
      },
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    await wrapper.get('[data-nav-key="/settings"]').trigger("click");

    expect(preferences.state.sidebarCollapsed).toBe(false);
  });
});
