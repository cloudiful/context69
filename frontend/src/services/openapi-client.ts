import createClient from "openapi-fetch";

import type { paths } from "../generated/openapi";
import { handleUnauthorized } from "./auth/session";

export const API_BASE_URL = resolveApiBaseUrl();

export const openapiClient = createClient<paths>({
  baseUrl: API_BASE_URL || undefined,
  credentials: "include",
});

openapiClient.use({
  async onResponse({ response }) {
    if (response.status === 401) handleUnauthorized();
    return response;
  },
});

function resolveApiBaseUrl(): string {
  const configured = (import.meta.env.VITE_API_BASE_URL || "").replace(/\/$/, "");
  if (configured) {
    return configured;
  }

  if (typeof window !== "undefined" && window.location?.origin) {
    return window.location.origin.replace(/\/$/, "");
  }

  return "";
}
