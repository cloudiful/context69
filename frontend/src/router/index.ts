import { createRouter, createWebHistory } from "vue-router";

import { authSessionState, ensureSessionReady, isAuthenticated, setAuthNavigator } from "../services/auth";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      redirect: "/search",
    },
    {
      path: "/login",
      name: "login",
      component: () => import("../views/LoginView.vue"),
      meta: {
        guestOnly: true,
      },
    },
    {
      path: "/search",
      name: "search",
      component: () => import("../views/SearchView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/groups",
      name: "groups",
      component: () => import("../views/GroupsView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/groups/:groupKey",
      component: () => import("../views/GroupShellView.vue"),
      meta: {
        requiresAuth: true,
      },
      children: [
        {
          path: "",
          name: "group-detail",
          redirect: (to) => ({
            name: "group-overview",
            params: to.params,
          }),
        },
        {
          path: "overview",
          name: "group-overview",
          component: () => import("../views/workspace/GroupOverviewPage.vue"),
        },
        {
          path: "projects",
          name: "group-projects",
          component: () => import("../views/workspace/GroupProjectsPage.vue"),
        },
        {
          path: "members",
          name: "group-members",
          component: () => import("../views/workspace/GroupMembersPage.vue"),
        },
      ],
    },
    {
      path: "/groups/:groupKey/projects/:projectKey",
      component: () => import("../views/ProjectShellView.vue"),
      meta: {
        requiresAuth: true,
      },
      children: [
        {
          path: "",
          name: "project",
          redirect: (to) => ({
            name: "project-overview",
            params: to.params,
          }),
        },
        {
          path: "overview",
          name: "project-overview",
          component: () => import("../views/workspace/ProjectOverviewPage.vue"),
        },
        {
          path: "sources",
          name: "project-sources",
          component: () => import("../views/workspace/ProjectSourcesPage.vue"),
        },
        {
          path: "files",
          name: "project-files",
          component: () => import("../views/workspace/ProjectFilesPage.vue"),
        },
        {
          path: "members",
          name: "project-members",
          component: () => import("../views/workspace/ProjectMembersPage.vue"),
        },
      ],
    },
    {
      path: "/sources",
      redirect: "/groups",
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings",
      redirect: "/settings/appearance",
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings/appearance",
      name: "settings-appearance",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings/access-tokens",
      name: "settings-access-tokens",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings/search",
      name: "settings-search",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings/runtime",
      name: "settings-runtime",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings/docling",
      name: "settings-docling",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/settings/admin-users",
      name: "settings-admin-users",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
        requiresAdmin: true,
      },
    },
    {
      path: "/library",
      redirect: "/groups",
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/documents/:id",
      name: "document",
      component: () => import("../views/DocumentView.vue"),
      props: true,
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/:pathMatch(.*)*",
      name: "not-found",
      component: () => import("../views/NotFoundView.vue"),
    },
  ],
  scrollBehavior() {
    return { top: 0 };
  },
});

setAuthNavigator((to) => {
  const current = router.currentRoute.value;
  const nextPath = to || current.fullPath;
  void router.replace({
    name: "login",
    query: nextPath && nextPath !== "/login"
      ? {
        redirect: nextPath,
        reason: authSessionState.lastFailureReason || "expired",
      }
      : {
        reason: authSessionState.lastFailureReason || "expired",
      },
  });
});

router.beforeEach(async (to) => {
  if (!authSessionState.ready) {
    await ensureSessionReady();
  }

  if (to.meta.requiresAuth && !isAuthenticated()) {
    return {
      name: "login",
      query: {
        redirect: to.fullPath,
        reason: authSessionState.lastFailureReason || "expired",
      },
    };
  }

  if (to.meta.guestOnly && isAuthenticated()) {
    return { name: "search" };
  }

  if (to.meta.requiresAdmin && !authSessionState.user?.is_admin) {
    return { name: "settings-appearance" };
  }

  return true;
});
