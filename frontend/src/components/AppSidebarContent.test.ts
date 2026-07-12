import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { useUiPreferences } from "../composables/use-ui-preferences";
import AppSidebarContent from "./AppSidebarContent.vue";

describe("AppSidebarContent", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setGuest();
  });

  it("renders business navigation with the current route highlighted", async () => {
    setAuthenticatedUser();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", component: { template: "<div />" } },
        { path: "/groups", component: { template: "<div />" } },
        { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div />" } },
        { path: "/groups/:groupPath/members", name: "group-members", component: { template: "<div />" } },
        { path: "/groups/:groupPath/settings", name: "group-settings", component: { template: "<div />" } },
        { path: "/settings", component: { template: "<div />" } },
        { path: "/settings/appearance", component: { template: "<div />" } },
        { path: "/settings/access-tokens", component: { template: "<div />" } },
        { path: "/settings/search", component: { template: "<div />" } },
      ],
    });

    router.push("/search");
    await router.isReady();

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
    expect(wrapper.findAll(".p-sidebar-menu-button svg")).toHaveLength(3);
    expect(wrapper.get('[aria-label="Log Out"]').attributes("aria-label")).toBe("Log Out");
  });

  it("renders settings secondary navigation when the settings section is active", async () => {
    setAuthenticatedUser();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", component: { template: "<div />" } },
        { path: "/groups", component: { template: "<div />" } },
        { path: "/settings", component: { template: "<div />" } },
        { path: "/settings/appearance", component: { template: "<div />" } },
        { path: "/settings/access-tokens", component: { template: "<div />" } },
        { path: "/settings/search", component: { template: "<div />" } },
        { path: "/settings/runtime", component: { template: "<div />" } },
        { path: "/settings/docling", component: { template: "<div />" } },
        { path: "/settings/admin-users", component: { template: "<div />" } },
      ],
    });

    router.push("/settings/access-tokens");
    await router.isReady();

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

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", component: { template: "<div />" } },
        { path: "/groups", component: { template: "<div />" } },
        { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div />" } },
        { path: "/groups/:groupPath/members", name: "group-members", component: { template: "<div />" } },
        { path: "/groups/:groupPath/settings", name: "group-settings", component: { template: "<div />" } },
        { path: "/settings", component: { template: "<div />" } },
      ],
    });

    router.push("/groups/stock/members");
    await router.isReady();

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
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/login", component: { template: "<div />" } },
      ],
    });

    router.push("/login");
    await router.isReady();

    const wrapper = mount(AppSidebarContent, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    await flushPromises();

    expect(wrapper.text()).not.toContain("Search");
    expect(wrapper.html()).not.toContain('aria-label="Log Out"');
  });

  it("centers the logout button and hides user details when collapsed", async () => {
    setAuthenticatedUser();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", component: { template: "<div />" } },
        { path: "/groups", component: { template: "<div />" } },
        { path: "/settings", component: { template: "<div />" } },
        { path: "/settings/appearance", component: { template: "<div />" } },
      ],
    });

    router.push("/search");
    await router.isReady();

    const wrapper = mount(AppSidebarContent, {
      props: {
        collapsed: true,
      },
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
      },
    });

    expect(wrapper.text()).not.toContain("Administrator");
    expect(wrapper.get(".p-sidebar-footer > div").classes()).toContain("justify-center");
    expect(wrapper.get('[aria-label="Log Out"]').attributes("aria-label")).toBe("Log Out");
  });

  it("expands the desktop sidebar when a collapsed top-level item has secondary navigation", async () => {
    setAuthenticatedUser();
    const preferences = useUiPreferences();
    preferences.state.sidebarCollapsed = true;

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", component: { template: "<div />" } },
        { path: "/groups", component: { template: "<div />" } },
        { path: "/settings", component: { template: "<div />" } },
        { path: "/settings/appearance", component: { template: "<div />" } },
        { path: "/settings/access-tokens", component: { template: "<div />" } },
      ],
    });

    router.push("/settings/appearance");
    await router.isReady();

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
