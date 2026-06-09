import { shallowReactive } from "vue";

import { API_BASE_URL } from "../openapi-client";
import type {
  ApiErrorResponse,
  AuthLoginRequest,
  AuthTokenResponse,
  AuthUserResponse,
} from "../api/api-types";

export type AuthStatus = "idle" | "restoring" | "authenticated" | "guest";

export type AuthFailureReason = "expired" | "invalid_credentials" | "network" | "unknown";

export class AuthError extends Error {
  status: number;
  reason: AuthFailureReason;

  constructor(message: string, status: number, reason: AuthFailureReason) {
    super(message);
    this.name = "AuthError";
    this.status = status;
    this.reason = reason;
  }
}

export interface AuthSessionState {
  status: AuthStatus;
  accessToken: string | null;
  user: AuthUserResponse | null;
  ready: boolean;
  lastFailureReason: AuthFailureReason | null;
}

export const authSessionState = shallowReactive<AuthSessionState>({
  status: "idle",
  accessToken: null,
  user: null,
  ready: false,
  lastFailureReason: null,
});

let restorePromise: Promise<boolean> | null = null;
let refreshPromise: Promise<boolean> | null = null;
let initialized = false;
let authNavigator: ((to?: string) => void) | null = null;

function resolveApiUrl(path: string): string {
  return API_BASE_URL ? `${API_BASE_URL}${path}` : path;
}

function readErrorMessage(fallback: string, payload: unknown) {
  if (payload && typeof payload === "object" && "error" in payload) {
    const message = (payload as ApiErrorResponse).error;
    if (typeof message === "string" && message) {
      return message;
    }
  }

  return fallback;
}

async function parseJson<T>(response: Response): Promise<T | null> {
  try {
    return await response.json() as T;
  } catch {
    return null;
  }
}

function applyIssuedSession(session: AuthTokenResponse) {
  authSessionState.accessToken = session.access_token;
  authSessionState.user = session.user;
  authSessionState.status = "authenticated";
  authSessionState.ready = true;
  authSessionState.lastFailureReason = null;
}

function setGuest(reason: AuthFailureReason | null = null) {
  authSessionState.accessToken = null;
  authSessionState.user = null;
  authSessionState.status = "guest";
  authSessionState.ready = true;
  authSessionState.lastFailureReason = reason;
}

async function issueSession(
  path: "/v1/auth/login" | "/v1/auth/refresh",
  init?: RequestInit,
): Promise<AuthTokenResponse> {
  let response: Response;

  try {
    response = await fetch(resolveApiUrl(path), {
      credentials: "include",
      headers: {
        Accept: "application/json",
        ...(init?.body ? { "Content-Type": "application/json" } : {}),
        ...(init?.headers || {}),
      },
      method: "POST",
      ...init,
    });
  } catch (error) {
    throw new AuthError(
      error instanceof Error ? error.message : "Network request failed",
      0,
      "network",
    );
  }

  const payload = await parseJson<AuthTokenResponse | ApiErrorResponse>(response);

  if (!response.ok) {
    const message = readErrorMessage(`Request failed with status ${response.status}`, payload);
    const reason: AuthFailureReason = response.status === 401
      ? path === "/v1/auth/login"
        ? "invalid_credentials"
        : "expired"
      : response.status >= 500
        ? "network"
        : "unknown";
    throw new AuthError(message, response.status, reason);
  }

  if (!payload || !("access_token" in payload)) {
    throw new AuthError("Authentication response was empty", response.status, "unknown");
  }

  return payload;
}

export function isAuthenticated() {
  return authSessionState.status === "authenticated" && !!authSessionState.accessToken;
}

export function getAccessToken() {
  return authSessionState.accessToken;
}

export function setAuthNavigator(navigate: (to?: string) => void) {
  authNavigator = navigate;
}

export function clearSession(reason: AuthFailureReason | null = null) {
  setGuest(reason);
}

export async function login(credentials: AuthLoginRequest) {
  const session = await issueSession("/v1/auth/login", {
    body: JSON.stringify(credentials),
  });
  applyIssuedSession(session);
  return session.user;
}

export async function logout() {
  try {
    await fetch(resolveApiUrl("/v1/auth/logout"), {
      credentials: "include",
      method: "POST",
    });
  } finally {
    setGuest(null);
  }
}

export async function refreshSession(): Promise<boolean> {
  if (refreshPromise) {
    return refreshPromise;
  }

  refreshPromise = (async () => {
    try {
      const session = await issueSession("/v1/auth/refresh");
      applyIssuedSession(session);
      return true;
    } catch (error) {
      const reason = error instanceof AuthError ? error.reason : "unknown";
      setGuest(reason);
      return false;
    } finally {
      refreshPromise = null;
    }
  })();

  return refreshPromise;
}

export async function restoreSession(): Promise<boolean> {
  if (restorePromise) {
    return restorePromise;
  }

  authSessionState.status = "restoring";
  authSessionState.ready = false;

  restorePromise = refreshSession().finally(() => {
    restorePromise = null;
  });

  return restorePromise;
}

export async function ensureSessionReady() {
  if (!initialized) {
    initialized = true;
    await restoreSession();
  } else if (!authSessionState.ready) {
    await restoreSession();
  }
}

export async function handleUnauthorized(to?: string) {
  const restored = await refreshSession();
  if (!restored) {
    authNavigator?.(to);
  }
  return restored;
}
