import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { groupWorkspaceStateKey } from "../../composables/group-workspace-context";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import GroupOverviewPage from "./GroupOverviewPage.vue";

function workspaceState(overrides: Record<string, unknown> = {}) {
  return {
    childGroups: [],
    childrenPage: { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } },
    childrenSearch: "",
    groupPath: "stock",
    group: { group_key: "stock", group_path: "stock", name: "Stock Group" },
    openCreateChildGroupDialog: vi.fn(),
    confirmDeleteChildGroup: vi.fn(),
    openEditChildGroupDialog: vi.fn(),
    openMoveChildGroupDialog: vi.fn(),
    openGroup: vi.fn(),
    changeChildrenPage: vi.fn(),
    changePageSize: vi.fn(),
    ...overrides,
  };
}

async function mountPage(path: string, state: Record<string, unknown>) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/groups", name: "groups", component: { template: "<div />" } },
      { path: "/search", name: "search", component: { template: "<div />" } },
      { path: "/groups/:groupPath", name: "group-overview", component: GroupOverviewPage },
    ],
  });
  await router.push(path);
  await router.isReady();
  const wrapper = mount(GroupOverviewPage, {
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n(), router],
      provide: {
        [groupWorkspaceStateKey as symbol]: state,
      },
      stubs: {
        ProjectFilesPanel: {
          props: ["groupName", "groupPath"],
          template: "<div class='panel-stub' :data-group-name='groupName' />",
        },
      },
    },
  });
  await flushPromises();
  return { router, wrapper };
}

describe("GroupOverviewPage", () => {
  it("always shows the return-to-groups control and passes the group name", async () => {
    const { wrapper } = await mountPage("/groups/stock", workspaceState());

    const back = wrapper.get('[data-testid="group-back-to-groups"]');
    expect(back.text()).toContain("Back to groups");
    expect(wrapper.find(".panel-stub").attributes("data-group-name")).toBe("Stock Group");
    wrapper.unmount();
  });

  it("navigates to the named groups route", async () => {
    const { router, wrapper } = await mountPage("/groups/stock", workspaceState());

    await wrapper.get('[data-testid="group-back-to-groups"]').trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("groups");
    wrapper.unmount();
  });

  it("keeps the conditional return-to-search control", async () => {
    const plain = await mountPage("/groups/stock", workspaceState());
    expect(plain.wrapper.find('[data-testid="group-back-to-search"]').exists()).toBe(false);
    plain.wrapper.unmount();

    const fromSearch = await mountPage("/groups/stock?from=search", workspaceState());
    expect(fromSearch.wrapper.find('[data-testid="group-back-to-search"]').exists()).toBe(true);
    expect(fromSearch.wrapper.find('[data-testid="group-back-to-groups"]').exists()).toBe(true);
    fromSearch.wrapper.unmount();
  });
});
