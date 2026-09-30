import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import App from "./App.vue";
import { createAppI18n } from "./i18n";
import { setAuthenticatedUser, setGuest } from "./test-utils/auth";
import { testNuxtUiPlugin } from "./test-utils/nuxt-ui";
import { installMockStorage } from "./test-utils/storage";

const shellRoutes = [
  { path: "/login", name: "login", component: { template: "<div>login-body</div>" } },
  {
    path: "/processing-queue",
    name: "processing-queue",
    component: { template: "<div>queue-body</div>" },
    meta: { contentLayout: "fill" },
  },
  { path: "/groups", name: "groups", component: { template: "<div>groups-body</div>" } },
  { path: "/settings", name: "settings", component: { template: "<div>settings-body</div>" } },
  { path: "/settings/:section", name: "settings-section", component: { template: "<div>settings-body</div>" } },
  {
    path: "/settings/appearance",
    name: "settings-appearance",
    component: { template: "<div>settings-body</div>" },
    meta: { contentLayout: "fill" },
  },
];

async function mountShell(path: string) {
  const router = createRouter({ history: createMemoryHistory(), routes: shellRoutes });
  await router.push(path);
  await router.isReady();

  const wrapper = mount(App, {
    global: { plugins: [testNuxtUiPlugin, router, createAppI18n("en")] },
  });
  await flushPromises();
  return wrapper;
}

describe("App shell", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    installMockStorage();
    setGuest();
  });

  it("hosts the route bar outside the scrollable content on header routes", async () => {
    setAuthenticatedUser();

    const wrapper = await mountShell("/processing-queue");

    expect(wrapper.find('[data-testid="sidebar-collapse-toggle"]').exists()).toBe(true);
    expect(wrapper.find('nav[aria-label="Primary"]').exists()).toBe(true);
    expect(wrapper.find("#app-route-actions").exists()).toBe(true);
    expect(wrapper.find("main #app-route-actions").exists()).toBe(false);
    expect(wrapper.get("main").classes()).toContain("overflow-hidden");
  });

  it("keeps the settings content host constrained", async () => {
    setAuthenticatedUser();

    const wrapper = await mountShell("/settings/appearance");

    expect(wrapper.find("#app-route-actions").exists()).toBe(true);
    expect(wrapper.get("main").classes()).toContain("overflow-hidden");
  });

  it("omits the route bar and lets plain routes flow", async () => {
    setAuthenticatedUser();

    const wrapper = await mountShell("/groups");

    expect(wrapper.find("#app-route-actions").exists()).toBe(false);
    expect(wrapper.get("main").classes()).not.toContain("overflow-hidden");
    expect(wrapper.find('[data-testid="sidebar-collapse-toggle"]').exists()).toBe(true);
  });

  it("keeps the login route outside the dashboard shell", async () => {
    setGuest();

    const wrapper = await mountShell("/login");

    expect(wrapper.text()).toContain("login-body");
    expect(wrapper.find("#app-route-actions").exists()).toBe(false);
    expect(wrapper.find('nav[aria-label="Primary"]').exists()).toBe(false);
  });
});
