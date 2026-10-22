/**
 * API origin resolution shared by the OpenAPI client and the auth store.
 *
 * Keeping it free of session logic avoids an import cycle between the typed
 * client and the store that reacts to unauthorized responses.
 */
export function resolveApiBaseUrl(): string {
  const configured = (import.meta.env.VITE_API_BASE_URL || "").replace(/\/$/, "");

  if (configured) {
    return configured;
  }

  if (typeof window !== "undefined" && window.location?.origin) {
    return window.location.origin.replace(/\/$/, "");
  }

  return "";
}

export const API_BASE_URL = resolveApiBaseUrl();

/** Prefixes a request path with the configured API base, matching the client. */
export function resolveApiUrl(path: string): string {
  return API_BASE_URL ? `${API_BASE_URL}${path}` : path;
}
