import type { Client } from "openapi-fetch";
import type { paths } from "../../generated/openapi";
import { handleUnauthorized } from "../../lib/auth-store";
import { openapiClient, resolveApiUrl } from "../openapi-client";
import type { ApiErrorCode, ApiErrorResponse, ApiResult, RequestOptions } from "./api-types";
import { createAdminUsersApi } from "./api-admin-users";
import { createDocumentResultsApi } from "./api-document-results";
import { createGitConnectionsApi } from "./api-git-connections";
import { createGitRepositoriesApi } from "./api-git-repositories";
import { createNamespacesApi } from "./api-namespaces";
import { createPersonalAccessTokensApi } from "./api-personal-access-tokens";
import { createGroupWorkspaceApi } from "./api-group-workspace";
import { createSearchApi } from "./api-search";
import { createSettingsApi } from "./api-settings";
import { createSourcesApi } from "./api-sources";
import { createTasksApi } from "./api-tasks";

export class ApiError extends Error {
  status: number;
  /** The server's stable machine-readable code, when the error envelope had one. */
  code?: ApiErrorCode;

  constructor(message: string, status: number, code?: ApiErrorCode) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

/** Narrows an error-envelope code to the generated vocabulary without guessing. */
function isApiErrorCode(value: string): value is ApiErrorCode {
  switch (value) {
    case "invalid_argument":
    case "unauthorized":
    case "forbidden":
    case "not_found":
    case "conflict":
    case "docling_submission_uncertain":
    case "payload_too_large":
    case "unprocessable_entity":
    case "rate_limited":
    case "upstream_error":
    case "unavailable":
    case "upstream_timeout":
    case "internal":
      return true;
    default:
      return false;
  }
}

function readErrorCode(error: unknown): ApiErrorCode | undefined {
  if (error && typeof error === "object" && "code" in error) {
    const code = error.code;
    if (typeof code === "string" && isApiErrorCode(code)) {
      return code;
    }
  }
  return undefined;
}

export type OpenApiClient = Client<paths>;

export { resolveApiUrl };

function readErrorMessage(response: Response, error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as ApiErrorResponse).message;
    if (typeof message === "string" && message) {
      return message;
    }
  }

  return `Request failed with status ${response.status}`;
}

export async function unwrapResponse<TData>(promise: Promise<ApiResult<TData>>): Promise<TData> {
  const { data, error, response } = await promise;

  if (!response.ok) {
    throw new ApiError(readErrorMessage(response, error), response.status, readErrorCode(error));
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

    throw new ApiError(readErrorMessage(response, error), response.status, readErrorCode(error));
  }

  if (response.status === 204) {
    return undefined as TData;
  }

  return response.json() as Promise<TData>;
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
  ...createGitConnectionsApi({ openapiClient, unwrapResponse }),
  ...createGitRepositoriesApi({ openapiClient, unwrapResponse }),
  ...createTasksApi({ openapiClient, unwrapResponse }),
  ...createSettingsApi({ openapiClient, unwrapResponse }),
  ...createSearchApi({ openapiClient, unwrapResponse }),
  ...createDocumentResultsApi({ openapiClient, unwrapResponse }),
  health(options?: RequestOptions) {
    return unwrapResponse(
      openapiClient.GET("/healthz", {
        signal: options?.signal,
      }),
    );
  },
};
