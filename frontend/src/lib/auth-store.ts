import { resolveApiUrl } from "./api-base";
import type { ApiErrorResponse, AuthLoginRequest, AuthMeResponse, AuthUserResponse } from "../services/api/api-types";

export type AuthStatus = "idle" | "restoring" | "authenticated" | "guest";
export type AuthFailureReason = "expired" | "invalid_credentials" | "network" | "unknown";

export interface AuthSessionSnapshot {
  status: AuthStatus;
  user: AuthUserResponse | null;
  ready: boolean;
  lastFailureReason: AuthFailureReason | null;
  restoreError: string | null;
}

export class AuthError extends Error {
  constructor(message: string, public status: number, public reason: AuthFailureReason) {
    super(message);
    this.name = "AuthError";
  }
}

const initialSnapshot: AuthSessionSnapshot = {
  status: "idle",
  user: null,
  ready: false,
  lastFailureReason: null,
  restoreError: null,
};

let snapshot: AuthSessionSnapshot = initialSnapshot;
const listeners = new Set<() => void>();
let restorePromise: Promise<boolean> | null = null;
let initialized = false;
let authNavigator: ((to?: string) => void) | null = null;

function emit(next: AuthSessionSnapshot) {
  snapshot = next;
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): AuthSessionSnapshot {
  return snapshot;
}

function setAuthenticated(user: AuthUserResponse) {
  emit({
    status: "authenticated",
    user,
    ready: true,
    lastFailureReason: null,
    restoreError: null,
  });
}

function setGuest(reason: AuthFailureReason | null = null) {
  emit({
    status: "guest",
    user: null,
    ready: true,
    lastFailureReason: reason,
    restoreError: null,
  });
}

async function parseJson<T>(response: Response): Promise<T | null> {
  try {
    return await response.json() as T;
  } catch {
    return null;
  }
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
  return snapshot.status === "authenticated" && !!snapshot.user;
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
      payload?.message || `Request failed with status ${response.status}`,
      response.status,
      response.status === 401 ? "invalid_credentials" : response.status >= 500 ? "network" : "unknown",
    );
  }

  if (!await restoreSession()) {
    throw new AuthError("Session was not available after login", 401, "expired");
  }

  return snapshot.user!;
}

export async function logout() {
  try {
    await authRequest("/v1/auth/logout", { method: "POST" });
  } finally {
    setGuest(null);
  }
}

export async function restoreSession(): Promise<boolean> {
  if (restorePromise) {
    return restorePromise;
  }

  emit({ ...snapshot, status: "restoring", ready: false, restoreError: null });

  restorePromise = (async () => {
    const response = await authRequest("/v1/auth/me");

    if (response.status === 401) {
      setGuest("expired");
      return false;
    }

    if (!response.ok) {
      emit({
        ...snapshot,
        status: snapshot.user ? "authenticated" : "idle",
        ready: true,
        lastFailureReason: "network",
        restoreError: `Authentication service returned ${response.status}`,
      });
      return false;
    }

    const payload = await parseJson<AuthMeResponse>(response);

    if (!payload?.user) {
      emit({
        ...snapshot,
        status: "idle",
        ready: true,
        lastFailureReason: "unknown",
        restoreError: "Authentication response was empty",
      });
      return false;
    }

    setAuthenticated(payload.user);
    return true;
  })().catch((error) => {
    emit({
      ...snapshot,
      status: snapshot.user ? "authenticated" : "idle",
      ready: true,
      lastFailureReason: error instanceof AuthError ? error.reason : "network",
      restoreError: error instanceof Error ? error.message : "Authentication service unavailable",
    });
    return false;
  }).finally(() => {
    restorePromise = null;
  });

  return restorePromise;
}

export async function ensureSessionReady() {
  if (!initialized || !snapshot.ready) {
    initialized = true;
    await restoreSession();
  }
}

/** Marks the session as expired and sends the visitor back to the login page. */
export function handleUnauthorized(to?: string) {
  setGuest("expired");
  authNavigator?.(to);
}

export const authSessionStore = {
  subscribe,
  getSnapshot,
};
