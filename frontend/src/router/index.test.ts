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
  },
  ensureSessionReady: vi.fn(),
  isAuthenticated: vi.fn(),
  setAuthNavigator: vi.fn(),
}));

vi.mock("../services/auth", () => ({
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
    authSessionState.ready = true;
    authSessionState.lastFailureReason = null;
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

    const { router } = await import("./index");

    await router.push("/settings");

    expect(ensureSessionReady).toHaveBeenCalledTimes(1);
    expect(router.currentRoute.value.name).toBe("settings");
  });
});
