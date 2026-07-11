import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  authSessionState,
  clearSession,
  login,
  restoreSession,
} from "./session";

const user = {
  user_id: 1,
  login_name: "admin",
  display_name: "Administrator",
  is_admin: true,
  disabled_at: null,
  personal_group_path: "personal-admin",
  personal_group_role: "owner" as const,
};

describe("browser auth session", () => {
  beforeEach(() => {
    clearSession(null);
    authSessionState.ready = false;
    authSessionState.restoreError = null;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("restores the user from the HttpOnly cookie session", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(restoreSession()).resolves.toBe(true);

    expect(authSessionState.user).toEqual(user);
    expect(fetchMock).toHaveBeenCalledWith(expect.stringMatching(/\/v1\/auth\/me$/), expect.objectContaining({ credentials: "include" }));
  });

  it("clears authentication only for a definitive unauthorized response", async () => {
    authSessionState.user = user;
    authSessionState.status = "authenticated";
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 401 })));

    await expect(restoreSession()).resolves.toBe(false);

    expect(authSessionState.user).toBeNull();
    expect(authSessionState.lastFailureReason).toBe("expired");
  });

  it("preserves the current user when session infrastructure is unavailable", async () => {
    authSessionState.user = user;
    authSessionState.status = "authenticated";
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 503 })));

    await expect(restoreSession()).resolves.toBe(false);

    expect(authSessionState.user).toEqual(user);
    expect(authSessionState.status).toBe("authenticated");
    expect(authSessionState.restoreError).toContain("503");
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
});
