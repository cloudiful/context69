import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  authSessionState,
  ensureSessionReady,
  isAuthenticated,
  setAuthNavigator,
} = vi.hoisted(() => ({
  authSessionState: {
    ready: true,
    lastFailureReason: null,
    user: null,
  } as {
    ready: boolean;
    lastFailureReason: string | null;
    user: { is_admin: boolean } | null;
  },
  ensureSessionReady: vi.fn(),
  isAuthenticated: vi.fn(),
  setAuthNavigator: vi.fn(),
}));

vi.mock("../services/auth/session", () => ({
  authSessionState,
  ensureSessionReady,
  isAuthenticated,
  setAuthNavigator,
}));

describe("router auth guards", () => {
  beforeEach(() => {
    vi.resetModules();
    ensureSessionReady.mockReset();
    isAuthenticated.mockReset();
    Object.defineProperty(document, "queryCommandSupported", {
      configurable: true,
      value: vi.fn(() => false),
    });
    authSessionState.ready = true;
    authSessionState.lastFailureReason = null;
    authSessionState.user = null;
  });

  it("redirects unauthenticated visitors to login with a redirect target", async () => {
    isAuthenticated.mockReturnValue(false);

    const { router } = await import("./index");

    await router.push("/search");

    expect(router.currentRoute.value.name).toBe("login");
    expect(router.currentRoute.value.query.redirect).toBe("/search");
  });

  it("redirects authenticated visitors away from the login page", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/login");

    expect(router.currentRoute.value.name).toBe("search");
  });

  it("restores the session before evaluating guards when app state is not ready", async () => {
    authSessionState.ready = false;
    isAuthenticated.mockReturnValue(true);
    authSessionState.user = {
      is_admin: true,
    };

    const { router } = await import("./index");

    await router.push("/settings");

    expect(ensureSessionReady).toHaveBeenCalledTimes(1);
    expect(router.currentRoute.value.name).toBe("settings-appearance");
  });

  it("redirects a group root route to the group overview page", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock");

    expect(router.currentRoute.value.name).toBe("group-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/overview");
  });

  it("redirects a project root route to the project overview page", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock/projects/alpha");

    expect(router.currentRoute.value.name).toBe("project-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/projects/alpha/overview");
  });

  it("redirects the group projects route to the group overview page", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock/projects");

    expect(router.currentRoute.value.name).toBe("group-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/overview");
  });

  it("keeps the group settings route addressable", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock/settings");

    expect(router.currentRoute.value.name).toBe("group-settings");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/settings");
  });

  it("keeps the project settings route addressable", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock/projects/alpha/settings");

    expect(router.currentRoute.value.name).toBe("project-settings");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/projects/alpha/settings");
  });

  it("redirects legacy project files and sources routes to the project overview page", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock/projects/alpha/files");

    expect(router.currentRoute.value.name).toBe("project-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/projects/alpha/overview");

    await router.push("/groups/stock/projects/alpha/sources");

    expect(router.currentRoute.value.name).toBe("project-overview");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/projects/alpha/overview");
  });
});
