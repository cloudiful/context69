import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import GroupShellView from "./GroupShellView.vue";
import ProjectShellView from "./ProjectShellView.vue";
import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";

const groupResponse = {
  created_at: "2026-01-01T00:00:00Z",
  current_role: "owner" as const,
  group_id: 1,
  group_key: "stock",
  kind: "shared" as const,
  name: "Stock Team",
  parent_group_key: null,
  updated_at: "2026-01-02T00:00:00Z",
  visibility: "private" as const,
};

const groupMembersResponse = [
  {
    user_id: 10,
    login_name: "alice",
    display_name: "Alice",
    role: "owner" as const,
  },
];

const projectsResponse = [
  {
    current_role: "owner" as const,
    group_key: "stock",
    name: "Alpha Project",
    project_id: 11,
    project_key: "alpha",
    updated_at: "2026-01-03T00:00:00Z",
    visibility: "private" as const,
  },
];

const projectMembersResponse = [
  {
    user_id: 10,
    login_name: "alice",
    display_name: "Alice",
    role: "owner" as const,
  },
];

function createApiSpies() {
  return {
    getGroup: vi.spyOn(apiClient, "getGroup").mockResolvedValue(groupResponse as never),
    listGroupMembers: vi.spyOn(apiClient, "listGroupMembers").mockResolvedValue(groupMembersResponse as never),
    listProjects: vi.spyOn(apiClient, "listProjects").mockResolvedValue(projectsResponse as never),
    listGroups: vi.spyOn(apiClient, "listGroups").mockResolvedValue([groupResponse] as never),
    getProject: vi.spyOn(apiClient, "getProject").mockResolvedValue(projectsResponse[0] as never),
    listProjectMembers: vi.spyOn(apiClient, "listProjectMembers").mockResolvedValue(projectMembersResponse as never),
  };
}

describe("workspace shells", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    createApiSpies();
  });

  it("redirects group routes to overview and keeps projects and members on dedicated pages", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        {
          path: "/groups/:groupKey",
          component: GroupShellView,
          children: [
            { path: "", name: "group-detail", redirect: (to) => ({ name: "group-overview", params: to.params }) },
            { path: "overview", name: "group-overview", component: () => import("./workspace/GroupOverviewPage.vue") },
            { path: "projects", name: "group-projects", component: () => import("./workspace/GroupProjectsPage.vue") },
            { path: "members", name: "group-members", component: () => import("./workspace/GroupMembersPage.vue") },
          ],
        },
      ],
    });

    router.push("/groups/stock");
    await router.isReady();

    const wrapper = mount(GroupShellView, {
      global: {
        plugins: [testPrimeVuePlugin, router, createTestI18n("en")],
      },
    });

    await flushPromises();

    expect(router.currentRoute.value.name).toBe("group-overview");
    expect(wrapper.text()).toContain("Stock Team");
    expect(wrapper.text()).not.toContain("Project Key");
    expect(wrapper.text()).not.toContain("Login Name");

    await router.push("/groups/stock/projects");
    await flushPromises();
    expect(wrapper.text()).toContain("Project Key");

    await router.push("/groups/stock/members");
    await flushPromises();
    expect(wrapper.text()).toContain("Login Name");
  });

  it("redirects project routes to overview and renders members as a dedicated route", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        {
          path: "/groups/:groupKey/projects/:projectKey",
          component: ProjectShellView,
          children: [
            { path: "", name: "project", redirect: (to) => ({ name: "project-overview", params: to.params }) },
            { path: "overview", name: "project-overview", component: () => import("./workspace/ProjectOverviewPage.vue") },
            { path: "sources", name: "project-sources", component: () => import("./workspace/ProjectSourcesPage.vue") },
            { path: "files", name: "project-files", component: () => import("./workspace/ProjectFilesPage.vue") },
            { path: "members", name: "project-members", component: () => import("./workspace/ProjectMembersPage.vue") },
          ],
        },
      ],
    });

    router.push("/groups/stock/projects/alpha");
    await router.isReady();

    const wrapper = mount(ProjectShellView, {
      global: {
        plugins: [testPrimeVuePlugin, router, createTestI18n("en")],
      },
    });

    await flushPromises();

    expect(router.currentRoute.value.name).toBe("project-overview");
    expect(wrapper.text()).toContain("Alpha Project");
    expect(wrapper.text()).not.toContain("Project Members");
    expect(wrapper.text()).not.toContain("SourcesFilesMembers");

    await router.push("/groups/stock/projects/alpha/members");
    await flushPromises();
    expect(wrapper.text()).toContain("Project Members");
  });
});
