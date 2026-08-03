import type { Client } from "openapi-fetch";
import type { paths } from "../../generated/openapi";
import { handleUnauthorized } from "../auth/session";
import { API_BASE_URL, openapiClient } from "../openapi-client";
import type { ApiErrorResponse, ApiResult, RequestOptions } from "./api-types";
import { createAdminUsersApi } from "./api-admin-users";
import { createLibraryApi } from "./api-library";
import { createNamespacesApi } from "./api-namespaces";
import { createPersonalAccessTokensApi } from "./api-personal-access-tokens";
import { createGroupWorkspaceApi } from "./api-group-workspace";
import { createSearchApi } from "./api-search";
import { createSettingsApi } from "./api-settings";
import { createSourcesApi } from "./api-sources";
import { createTasksApi } from "./api-tasks";

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
  const response = await fetch(input, { ...init, credentials: "include" });
  if (response.status === 401) handleUnauthorized();
  return response;
}

export const apiClient = {
  ...createAdminUsersApi({ openapiClient, unwrapResponse }),
  ...createNamespacesApi({ openapiClient, unwrapResponse }),
  ...createPersonalAccessTokensApi({ openapiClient, unwrapResponse }),
  ...createGroupWorkspaceApi({
    authFetch,
    openapiClient,
    resolveApiUrl,
    unwrapFetchResponse,
    unwrapResponse,
  }),
  ...createSourcesApi({ openapiClient, unwrapResponse }),
  ...createTasksApi({ openapiClient, unwrapResponse }),
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
