import type { GitIndexStatus, GitRepositorySource } from "@/services/api";

export const GIT_ACTIVE_POLL_MS = 3000;

export type GitSourcePollingState = Pick<
  GitRepositorySource,
  "index_status" | "refresh_pending" | "deletion_pending"
>;

export interface GitSourcesQueryState {
  state: {
    data: GitSourcePollingState[] | undefined;
  };
}

export function gitSourcesRefetchInterval(query: GitSourcesQueryState): number | false {
  const active = query.state.data?.some((source) => (
    source.index_status === "indexing"
    || source.index_status === "pending"
    || source.refresh_pending
    || source.deletion_pending
  ));
  return active ? GIT_ACTIVE_POLL_MS : false;
}

export function gitFilesRefetchInterval(status: GitIndexStatus): number | false {
  return status === "indexing" || status === "pending" ? GIT_ACTIVE_POLL_MS : false;
}
