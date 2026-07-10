import type {
  CreateGroupRequest,
  MoveGroupRequest,
  RequestOptions,
  UpdateGroupRequest,
  UpsertMembershipRequest,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createNamespacesApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listGroups(options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups", { signal: options?.signal }));
    },
    createGroup(payload: CreateGroupRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups", { body: payload, signal: options?.signal }));
    },
    getGroup(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}", {
        params: { path: { group_path: groupPath } },
        signal: options?.signal,
      }));
    },
    updateGroup(groupPath: string, payload: UpdateGroupRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PATCH("/v1/groups/by-path/{group_path}", {
        params: { path: { group_path: groupPath } },
        body: payload,
        signal: options?.signal,
      }));
    },
    moveGroup(groupPath: string, payload: MoveGroupRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/move", {
        params: { path: { group_path: groupPath } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteGroup(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/by-path/{group_path}", {
        params: { path: { group_path: groupPath } },
        signal: options?.signal,
      }));
    },
    listChildGroups(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/children", {
        params: { path: { group_path: groupPath } },
        signal: options?.signal,
      }));
    },
    listGroupMembers(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/members", {
        params: { path: { group_path: groupPath } },
        signal: options?.signal,
      }));
    },
    upsertGroupMember(groupPath: string, payload: UpsertMembershipRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/members", {
        params: { path: { group_path: groupPath } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteGroupMember(groupPath: string, loginName: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/by-path/{group_path}/members/{login_name}", {
        params: { path: { group_path: groupPath, login_name: loginName } },
        signal: options?.signal,
      }));
    },
    searchUserDirectory(query: string, limit = 10, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/user-directory", {
        params: { query: { query, limit } },
        signal: options?.signal,
      }));
    },
  };
}
