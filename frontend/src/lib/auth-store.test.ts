import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  authSessionStore,
  clearSession,
  handleUnauthorized,
  isAuthenticated,
  login,
  logout,
  restoreSession,
  setAuthNavigator,
} from "./auth-store";

const user = {
  user_id: 1,
  login_name: "admin",
  display_name: "Administrator",
  is_admin: true,
  disabled_at: null,
  personal_group_path: "personal-admin",
  personal_group_role: "owner" as const,
};

describe("neutral auth session store", () => {
  beforeEach(() => {
    clearSession(null);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAuthNavigator(() => undefined);
  });

  it("restores the user from the HttpOnly cookie session", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(restoreSession()).resolves.toBe(true);

    expect(authSessionStore.getSnapshot().user).toEqual(user);
    expect(isAuthenticated()).toBe(true);
    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringMatching(/\/v1\/auth\/me$/),
      expect.objectContaining({ credentials: "include" }),
    );
  });

  it("publishes each state change to store subscribers", async () => {
    const listener = vi.fn();
    const unsubscribe = authSessionStore.subscribe(listener);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })));

    await restoreSession();

    expect(listener).toHaveBeenCalled();
    expect(authSessionStore.getSnapshot().status).toBe("authenticated");

    unsubscribe();
    listener.mockClear();
    clearSession("expired");
    expect(listener).not.toHaveBeenCalled();
  });

  it("clears authentication only for a definitive unauthorized response", async () => {
    clearSession(null);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })));
    await restoreSession();

    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 401 })));

    await expect(restoreSession()).resolves.toBe(false);

    expect(authSessionStore.getSnapshot().user).toBeNull();
    expect(authSessionStore.getSnapshot().lastFailureReason).toBe("expired");
  });

  it("preserves the current user when session infrastructure is unavailable", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })));
    await restoreSession();

    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 503 })));

    await expect(restoreSession()).resolves.toBe(false);

    const snapshot = authSessionStore.getSnapshot();
    expect(snapshot.user).toEqual(user);
    expect(snapshot.status).toBe("authenticated");
    expect(snapshot.restoreError).toContain("503");
  });

  it("reports a network failure without inventing a session", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));

    await expect(restoreSession()).resolves.toBe(false);

    const snapshot = authSessionStore.getSnapshot();
    expect(snapshot.user).toBeNull();
    expect(snapshot.lastFailureReason).toBe("network");
    expect(snapshot.restoreError).toBe("Failed to fetch");
  });

  it("logs in with a cookie request and then loads the current user", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ user }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }));
    vi.stubGlobal("fetch", fetchMock);
    const storageSpy = vi.spyOn(Storage.prototype, "setItem");

    await expect(login({ login_name: "admin", password: "secret" })).resolves.toEqual(user);

    expect(fetchMock).toHaveBeenNthCalledWith(1, expect.stringMatching(/\/v1\/auth\/login$/), expect.objectContaining({
      credentials: "include",
      method: "POST",
    }));
    expect(storageSpy).not.toHaveBeenCalled();
  });

  it("maps a rejected credential response to an invalid-credentials error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ message: "Nope" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    })));

    await expect(login({ login_name: "admin", password: "wrong" })).rejects.toMatchObject({
      name: "AuthError",
      status: 401,
      reason: "invalid_credentials",
    });
  });

  it("clears the session even when the logout request fails", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })));
    await restoreSession();

    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));

    await expect(logout()).rejects.toThrow("Failed to fetch");
    expect(authSessionStore.getSnapshot().status).toBe("guest");
    expect(authSessionStore.getSnapshot().user).toBeNull();
  });

  it("sends an expired session to the registered navigator", () => {
    const navigate = vi.fn();
    setAuthNavigator(navigate);

    handleUnauthorized("/settings");

    expect(navigate).toHaveBeenCalledWith("/settings");
    expect(authSessionStore.getSnapshot().status).toBe("guest");
    expect(authSessionStore.getSnapshot().lastFailureReason).toBe("expired");
  });
});
