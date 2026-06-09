import type {
  CreateAdminUserRequest,
  RequestOptions,
  ResetAdminUserPasswordRequest,
  UpdateAdminUserRequest,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createAdminUsersApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listAdminUsers(options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/admin/users", { signal: options?.signal }));
    },
    createAdminUser(payload: CreateAdminUserRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/admin/users", { body: payload, signal: options?.signal }));
    },
    updateAdminUser(loginName: string, payload: UpdateAdminUserRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PATCH("/v1/admin/users/{login_name}", {
        params: { path: { login_name: loginName } },
        body: payload,
        signal: options?.signal,
      }));
    },
    resetAdminUserPassword(loginName: string, payload: ResetAdminUserPasswordRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/admin/users/{login_name}/reset-password", {
        params: { path: { login_name: loginName } },
        body: payload,
        signal: options?.signal,
      }));
    },
    disableAdminUser(loginName: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/admin/users/{login_name}/disable", {
        params: { path: { login_name: loginName } },
        signal: options?.signal,
      }));
    },
    enableAdminUser(loginName: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/admin/users/{login_name}/enable", {
        params: { path: { login_name: loginName } },
        signal: options?.signal,
      }));
    },
  };
}
