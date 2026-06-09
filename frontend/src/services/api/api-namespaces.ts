import type {
  CreateGroupRequest,
  CreateProjectRequest,
  MoveProjectRequest,
  RequestOptions,
  UpdateGroupRequest,
  UpdateProjectRequest,
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
    getGroup(groupKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}", {
        params: { path: { group_key: groupKey } },
        signal: options?.signal,
      }));
    },
    updateGroup(groupKey: string, payload: UpdateGroupRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PATCH("/v1/groups/{group_key}", {
        params: { path: { group_key: groupKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteGroup(groupKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}", {
        params: { path: { group_key: groupKey } },
        signal: options?.signal,
      }));
    },
    listGroupMembers(groupKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/members", {
        params: { path: { group_key: groupKey } },
        signal: options?.signal,
      }));
    },
    upsertGroupMember(groupKey: string, payload: UpsertMembershipRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/members", {
        params: { path: { group_key: groupKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteGroupMember(groupKey: string, loginName: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}/members/{login_name}", {
        params: { path: { group_key: groupKey, login_name: loginName } },
        signal: options?.signal,
      }));
    },
    listProjects(groupKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects", {
        params: { path: { group_key: groupKey } },
        signal: options?.signal,
      }));
    },
    createProject(groupKey: string, payload: CreateProjectRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects", {
        params: { path: { group_key: groupKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    getProject(groupKey: string, projectKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects/{project_key}", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        signal: options?.signal,
      }));
    },
    updateProject(groupKey: string, projectKey: string, payload: UpdateProjectRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PATCH("/v1/groups/{group_key}/projects/{project_key}", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteProject(groupKey: string, projectKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}/projects/{project_key}", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        signal: options?.signal,
      }));
    },
    moveProject(groupKey: string, projectKey: string, payload: MoveProjectRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/move", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    listProjectMembers(groupKey: string, projectKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects/{project_key}/members", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        signal: options?.signal,
      }));
    },
    upsertProjectMember(groupKey: string, projectKey: string, payload: UpsertMembershipRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/members", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteProjectMember(groupKey: string, projectKey: string, loginName: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}/projects/{project_key}/members/{login_name}", {
        params: { path: { group_key: groupKey, project_key: projectKey, login_name: loginName } },
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
