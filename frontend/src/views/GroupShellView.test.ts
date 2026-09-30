import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { setAuthenticatedUser } from "../test-utils/auth";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import GroupShellView from "./GroupShellView.vue";

const getGroup = vi.spyOn(apiClient, "getGroup");
const listGroupMembers = vi.spyOn(apiClient, "listGroupMembers");
const listChildGroups = vi.spyOn(apiClient, "listChildGroups");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
const useToast = vi.spyOn(nuxtUiComposables, "useToast");

const groupResponse = {
  group_key: "stock",
  group_path: "stock",
  name: "Stock",
  visibility: "private",
  kind: "shared",
  current_role: "owner",
  created_at: "2026-01-01T00:00:00Z",
};

const emptyPage = { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } };

const Host = { template: "<RouterView />" };

async function mountShell(path = "/groups/stock") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: "/groups/:groupPath",
        component: GroupShellView,
        children: [
          { path: "", name: "group-overview", component: { template: "<div>overview</div>" } },
          { path: "members", name: "group-members", component: { template: "<div>members</div>" } },
          { path: "settings", name: "group-settings", component: { template: "<div>settings</div>" } },
        ],
      },
    ],
  });

  await router.push(path);
  await router.isReady();

  const wrapper = mount(Host, {
    global: { plugins: [testNuxtUiPlugin, createTestI18n(), router] },
  });
  await flushPromises();
  return { router, wrapper };
}

describe("GroupShellView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setAuthenticatedUser();
    getGroup.mockReset().mockResolvedValue(groupResponse as never);
    listGroupMembers.mockReset().mockResolvedValue(emptyPage as never);
    listChildGroups.mockReset().mockResolvedValue(emptyPage as never);
    useOverlay.mockReset().mockReturnValue({
      create: () => ({ open: async () => true }),
    } as never);
    useToast.mockReset().mockReturnValue({ add: vi.fn() } as never);
  });

  it("stacks the mobile section tabs above a bounded content region", async () => {
    const { wrapper } = await mountShell();

    const root = wrapper.get('[data-testid="group-shell"]');
    expect(root.classes()).toContain("flex");
    expect(root.classes()).toContain("flex-col");
    expect(root.classes()).toContain("h-full");
    expect(root.classes()).toContain("min-h-0");
    // The tabs must not stretch into a 1fr grid row and push the page content
    // off screen.
    expect(root.attributes("class") ?? "").not.toContain("grid-rows-");

    const content = wrapper.get('[data-testid="group-section-content"]');
    expect(content.classes()).toContain("flex-1");
    expect(content.classes()).toContain("min-h-0");

    expect(wrapper.find('[data-slot="trigger"]').exists()).toBe(true);
  });

  it("routes between group sections from the mobile tab bar", async () => {
    const { router, wrapper } = await mountShell();

    expect(router.currentRoute.value.name).toBe("group-overview");

    const triggers = wrapper.findAll('[data-slot="trigger"]');
    expect(triggers.map((trigger) => trigger.text())).toEqual(["Overview", "Members", "Settings"]);

    const members = triggers.find((trigger) => trigger.text() === "Members");
    expect(members).toBeDefined();
    await members!.trigger("mousedown");
    await flushPromises();
    expect(router.currentRoute.value.name).toBe("group-members");

    const settings = wrapper.findAll('[data-slot="trigger"]').find((trigger) => trigger.text() === "Settings");
    expect(settings).toBeDefined();
    await settings!.trigger("mousedown");
    await flushPromises();
    expect(router.currentRoute.value.name).toBe("group-settings");
  });

  it("marks the active group section and keeps nested group routes addressable", async () => {
    const { router, wrapper } = await mountShell("/groups/stock/members");

    const active = wrapper.findAll('[data-slot="trigger"]').find((trigger) => trigger.text() === "Members");
    expect(active).toBeDefined();
    expect(active!.attributes("data-state")).toBe("active");

    const overview = wrapper.findAll('[data-slot="trigger"]').find((trigger) => trigger.text() === "Overview");
    expect(overview!.attributes("data-state")).not.toBe("active");

    expect(router.currentRoute.value.name).toBe("group-members");
  });
});