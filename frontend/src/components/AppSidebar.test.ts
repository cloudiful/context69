import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import Sidebar from "primevue/sidebar";

import { createAppI18n } from "../i18n";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { useUiPreferences } from "../composables/use-ui-preferences";
import AppSidebar from "./AppSidebar.vue";

describe("AppSidebar", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setGuest();
    const preferences = useUiPreferences();
    preferences.state.sidebarCollapsed = false;
    preferences.state.mobileNavOpen = false;
  });

  it("renders PrimeVue sidebars instead of the old aside shell", async () => {
    setAuthenticatedUser();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: { template: "<div />" } },
      ],
    });

    router.push("/search");
    await router.isReady();

    const wrapper = mount(AppSidebar, {
      global: {
        plugins: [testPrimeVuePlugin, router, createAppI18n("en")],
        stubs: {
          teleport: true,
        },
      },
    });

    expect(wrapper.findAllComponents(Sidebar)).toHaveLength(2);
    expect(wrapper.find("aside.app-sidebar").exists()).toBe(false);
  });

});
