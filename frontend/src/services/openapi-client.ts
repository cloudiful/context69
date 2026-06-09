import createClient from "openapi-fetch";

import type { paths } from "../generated/openapi";
import { getAccessToken, handleUnauthorized } from "./auth";

export const API_BASE_URL = resolveApiBaseUrl();

export const openapiClient = createClient<paths>({
  baseUrl: API_BASE_URL || undefined,
});

openapiClient.use({
  async onRequest({ request }) {
    const token = getAccessToken();
    if (token) {
      request.headers.set("Authorization", `Bearer ${token}`);
    }
    return request;
  },
  async onResponse({ request, response }) {
    if (response.status !== 401) {
      return response;
    }

    if (request.headers.get("x-context69-retry") === "1") {
      return response;
    }

    const restored = await handleUnauthorized();
    if (!restored) {
      return response;
    }

    const retryRequest = new Request(request);
    retryRequest.headers.set("x-context69-retry", "1");

    const token = getAccessToken();
    if (token) {
      retryRequest.headers.set("Authorization", `Bearer ${token}`);
    } else {
      retryRequest.headers.delete("Authorization");
    }

    return fetch(retryRequest);
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
