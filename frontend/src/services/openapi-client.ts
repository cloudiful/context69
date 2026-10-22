import createClient from "openapi-fetch";

import type { paths } from "../generated/openapi";
import { API_BASE_URL, resolveApiUrl } from "../lib/api-base";
import { handleUnauthorized } from "../lib/auth-store";

export { API_BASE_URL, resolveApiUrl };

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
