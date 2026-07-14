import { beforeEach, describe, expect, it, vi } from "vitest";
import * as authSession from "../services/auth/session";
import { router } from "./index";

const { authSessionState } = authSession;
const ensureSessionReady = vi.spyOn(authSession, "ensureSessionReady");
const isAuthenticated = vi.spyOn(authSession, "isAuthenticated");

describe("router auth guards", () => {
  beforeEach(() => {
    ensureSessionReady.mockReset();
    isAuthenticated.mockReset();
    Object.defineProperty(document, "queryCommandSupported", {
      configurable: true,
      value: vi.fn(() => false),
    });
    authSessionState.ready = true;
    authSessionState.status = "guest";
    authSessionState.lastFailureReason = null;
    authSessionState.restoreError = null;
    authSessionState.user = null;
    window.scrollTo = vi.fn();
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
      user_id: 1,
      display_name: "Test User",
      login_name: "test-user",
      personal_group_path: "personal/test-user",
      is_admin: true,
    };

    const { router } = await import("./index");

    await router.push("/settings");

    expect(ensureSessionReady).toHaveBeenCalledTimes(1);
    expect(router.currentRoute.value.name).toBe("settings-appearance");
  });

  it("uses the group root route as the group overview page", async () => {
    const { router } = await import("./index");
    const resolved = router.resolve("/groups/stock");

    expect(resolved.name).toBe("group-overview");
    expect(resolved.fullPath).toBe("/groups/stock");
  });

  it("supports encoded nested group paths", async () => {
    const { router } = await import("./index");
    const resolved = router.resolve("/groups/stock%2Falpha");

    expect(resolved.name).toBe("group-overview");
    expect(resolved.fullPath).toBe("/groups/stock%2Falpha");
  });

  it("redirects legacy overview URLs to the group root", async () => {
    const { router } = await import("./index");
    const resolved = router.resolve("/groups/stock/overview");
    const redirect = resolved.matched.at(-1)?.redirect;

    expect(typeof redirect).toBe("function");
    expect((redirect as (to: typeof resolved) => unknown)(resolved)).toEqual({
      name: "group-overview",
      params: resolved.params,
    });
  });

  it("keeps the group settings route addressable", async () => {
    isAuthenticated.mockReturnValue(true);

    const { router } = await import("./index");

    await router.push("/groups/stock/settings");

    expect(router.currentRoute.value.name).toBe("group-settings");
    expect(router.currentRoute.value.fullPath).toBe("/groups/stock/settings");
  });

});
