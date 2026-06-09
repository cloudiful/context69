import type { Client } from "openapi-fetch";

import type { paths } from "../../generated/openapi";
import { getAccessToken, handleUnauthorized } from "../auth";
import { API_BASE_URL, openapiClient } from "../openapi-client";
import type { ApiErrorResponse, ApiResult, RequestOptions } from "./api-types";
import { createAdminUsersApi } from "./api-admin-users";
import { createLibraryApi } from "./api-library";
import { createNamespacesApi } from "./api-namespaces";
import { createProjectWorkspaceApi } from "./api-project-workspace";
import { createSearchApi } from "./api-search";
import { createSettingsApi } from "./api-settings";
import { createSourcesApi } from "./api-sources";

export class ApiError extends Error {
  status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

export type OpenApiClient = Client<paths>;

function readErrorMessage(response: Response, error: unknown): string {
  if (error && typeof error === "object" && "error" in error) {
    const message = (error as ApiErrorResponse).error;
    if (typeof message === "string" && message) {
      return message;
    }
  }

  return `Request failed with status ${response.status}`;
}

export async function unwrapResponse<TData>(promise: Promise<ApiResult<TData>>): Promise<TData> {
  const { data, error, response } = await promise;

  if (!response.ok) {
    throw new ApiError(readErrorMessage(response, error), response.status);
  }

  return data as TData;
}

export async function unwrapFetchResponse<TData>(response: Response): Promise<TData> {
  if (!response.ok) {
    let error: unknown = null;

    try {
      error = await response.json();
    } catch {
      error = null;
    }

    throw new ApiError(readErrorMessage(response, error), response.status);
  }

  if (response.status === 204) {
    return undefined as TData;
  }

  return response.json() as Promise<TData>;
}

export function resolveApiUrl(path: string): string {
  return API_BASE_URL ? `${API_BASE_URL}${path}` : path;
}

export async function authFetch(input: RequestInfo | URL, init?: RequestInit) {
  const request = new Request(input, init);
  const token = getAccessToken();
  if (token) {
    request.headers.set("Authorization", `Bearer ${token}`);
  }

  let response = await fetch(request);
  if (response.status !== 401 || request.headers.get("x-context69-retry") === "1") {
    return response;
  }

  const restored = await handleUnauthorized();
  if (!restored) {
    return response;
  }

  const retryRequest = new Request(request);
  retryRequest.headers.set("x-context69-retry", "1");
  const nextToken = getAccessToken();
  if (nextToken) {
    retryRequest.headers.set("Authorization", `Bearer ${nextToken}`);
  } else {
    retryRequest.headers.delete("Authorization");
  }

  response = await fetch(retryRequest);
  return response;
}

export const apiClient = {
  ...createAdminUsersApi({ openapiClient, unwrapResponse }),
  ...createNamespacesApi({ openapiClient, unwrapResponse }),
  ...createProjectWorkspaceApi({
    authFetch,
    openapiClient,
    resolveApiUrl,
    unwrapFetchResponse,
    unwrapResponse,
  }),
  ...createSourcesApi({ openapiClient, unwrapResponse }),
  ...createSettingsApi({ openapiClient, unwrapResponse }),
  ...createSearchApi({ openapiClient, unwrapResponse }),
  ...createLibraryApi({
    authFetch,
    openapiClient,
    resolveApiUrl,
    unwrapFetchResponse,
    unwrapResponse,
  }),
  health(options?: RequestOptions) {
    return unwrapResponse(
      openapiClient.GET("/healthz", {
        signal: options?.signal,
      }),
    );
  },
};
