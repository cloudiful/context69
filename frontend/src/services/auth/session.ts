import { shallowReactive } from "vue";

import { API_BASE_URL } from "../openapi-client";
import type { ApiErrorResponse, AuthLoginRequest, AuthMeResponse, AuthUserResponse } from "../api/api-types";

export type AuthStatus = "idle" | "restoring" | "authenticated" | "guest";
export type AuthFailureReason = "expired" | "invalid_credentials" | "network" | "unknown";

export class AuthError extends Error {
  constructor(message: string, public status: number, public reason: AuthFailureReason) {
    super(message);
    this.name = "AuthError";
  }
}

export const authSessionState = shallowReactive<{
  status: AuthStatus;
  user: AuthUserResponse | null;
  ready: boolean;
  lastFailureReason: AuthFailureReason | null;
  restoreError: string | null;
}>({
  status: "idle",
  user: null,
  ready: false,
  lastFailureReason: null,
  restoreError: null,
});

let restorePromise: Promise<boolean> | null = null;
let initialized = false;
let authNavigator: ((to?: string) => void) | null = null;

function resolveApiUrl(path: string) {
  return API_BASE_URL ? `${API_BASE_URL}${path}` : path;
}

async function parseJson<T>(response: Response): Promise<T | null> {
  try {
    return await response.json() as T;
  } catch {
    return null;
  }
}

function setAuthenticated(user: AuthUserResponse) {
  authSessionState.user = user;
  authSessionState.status = "authenticated";
  authSessionState.ready = true;
  authSessionState.lastFailureReason = null;
  authSessionState.restoreError = null;
}

function setGuest(reason: AuthFailureReason | null = null) {
  authSessionState.user = null;
  authSessionState.status = "guest";
  authSessionState.ready = true;
  authSessionState.lastFailureReason = reason;
  authSessionState.restoreError = null;
}

async function authRequest(path: string, init?: RequestInit) {
  try {
    return await fetch(resolveApiUrl(path), {
      credentials: "include",
      headers: {
        Accept: "application/json",
        ...(init?.body ? { "Content-Type": "application/json" } : {}),
        ...init?.headers,
      },
      ...init,
    });
  } catch (error) {
    throw new AuthError(error instanceof Error ? error.message : "Network request failed", 0, "network");
  }
}

export function isAuthenticated() {
  return authSessionState.status === "authenticated" && !!authSessionState.user;
}

export function setAuthNavigator(navigate: (to?: string) => void) {
  authNavigator = navigate;
}

export function clearSession(reason: AuthFailureReason | null = null) {
  setGuest(reason);
}

export async function login(credentials: AuthLoginRequest) {
  const response = await authRequest("/v1/auth/login", {
    method: "POST",
    body: JSON.stringify(credentials),
  });
  if (!response.ok) {
    const payload = await parseJson<ApiErrorResponse>(response);
    throw new AuthError(
      payload?.error || `Request failed with status ${response.status}`,
      response.status,
      response.status === 401 ? "invalid_credentials" : response.status >= 500 ? "network" : "unknown",
    );
  }
  if (!await restoreSession()) {
    throw new AuthError("Session was not available after login", 401, "expired");
  }
  return authSessionState.user!;
}

export async function logout() {
  try {
    await authRequest("/v1/auth/logout", { method: "POST" });
  } finally {
    setGuest(null);
  }
}

export async function restoreSession(): Promise<boolean> {
  if (restorePromise) return restorePromise;
  authSessionState.status = "restoring";
  authSessionState.ready = false;
  authSessionState.restoreError = null;
  restorePromise = (async () => {
    const response = await authRequest("/v1/auth/me");
    if (response.status === 401) {
      setGuest("expired");
      return false;
    }
    if (!response.ok) {
      authSessionState.status = authSessionState.user ? "authenticated" : "idle";
      authSessionState.ready = true;
      authSessionState.lastFailureReason = "network";
      authSessionState.restoreError = `Authentication service returned ${response.status}`;
      return false;
    }
    const payload = await parseJson<AuthMeResponse>(response);
    if (!payload?.user) {
      authSessionState.status = "idle";
      authSessionState.ready = true;
      authSessionState.lastFailureReason = "unknown";
      authSessionState.restoreError = "Authentication response was empty";
      return false;
    }
    setAuthenticated(payload.user);
    return true;
  })().catch((error) => {
    authSessionState.status = authSessionState.user ? "authenticated" : "idle";
    authSessionState.ready = true;
    authSessionState.lastFailureReason = error instanceof AuthError ? error.reason : "network";
    authSessionState.restoreError = error instanceof Error ? error.message : "Authentication service unavailable";
    return false;
  }).finally(() => {
    restorePromise = null;
  });
  return restorePromise;
}

export async function ensureSessionReady() {
  if (!initialized || !authSessionState.ready) {
    initialized = true;
    await restoreSession();
  }
}

export function handleUnauthorized(to?: string) {
  setGuest("expired");
  authNavigator?.(to);
}
