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
      name: "group-detail",
      component: () => import("../views/GroupDetailView.vue"),
      meta: {
        requiresAuth: true,
      },
    },
    {
      path: "/groups/:groupKey/projects/:projectKey",
      name: "project",
      component: () => import("../views/ProjectView.vue"),
      meta: {
        requiresAuth: true,
      },
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
      name: "settings",
      component: () => import("../views/SettingsView.vue"),
      meta: {
        requiresAuth: true,
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

  return true;
});
