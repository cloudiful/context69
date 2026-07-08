import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createAppI18n } from "../i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
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
        { path: "/settings", component: { template: "<div />" } },
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
    expect(wrapper.find('[data-nav-key="/search"]').classes()).toContain("is-active");
    expect(wrapper.findAll(".app-sidebar-link-icon")).toHaveLength(4);
    expect(wrapper.get('[aria-label="Log Out"]').attributes("aria-label")).toBe("Log Out");
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
    expect(wrapper.get(".app-sidebar-footer > div").classes()).toContain("justify-center");
    expect(wrapper.get('[aria-label="Log Out"]').attributes("aria-label")).toBe("Log Out");
  });
});
