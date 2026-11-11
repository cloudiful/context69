import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

import {
  GIT_ACTIVE_POLL_MS,
  gitFilesRefetchInterval,
  gitSourcesRefetchInterval,
  type GitSourcePollingState,
} from "./polling";

const idle: GitSourcePollingState = {
  index_status: "ready",
  refresh_pending: false,
  deletion_pending: false,
};

function sourcesQuery(data: GitSourcePollingState[] | undefined) {
  return { state: { data } };
}

describe("Git polling policy", () => {
  it("polls the source list while an index or durable demand is active", () => {
    const activeStates: GitSourcePollingState[] = [
      { ...idle, index_status: "indexing" },
      { ...idle, index_status: "pending" },
      { ...idle, refresh_pending: true },
      { ...idle, deletion_pending: true },
    ];

    for (const state of activeStates) {
      expect(gitSourcesRefetchInterval(sourcesQuery([state]))).toBe(GIT_ACTIVE_POLL_MS);
    }

    expect(gitSourcesRefetchInterval(sourcesQuery([idle]))).toBe(false);
    expect(gitSourcesRefetchInterval(sourcesQuery([idle, { ...idle, index_status: "failed" }]))).toBe(false);
    expect(gitSourcesRefetchInterval(sourcesQuery(undefined))).toBe(false);
  });

  it("keeps polling a query after more than sixty real data updates", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const queryKey = ["git-sources", "stock/alpha"];
    const queryFn = async (): Promise<GitSourcePollingState[]> => [
      { ...idle, index_status: "indexing" },
    ];

    for (let update = 0; update < 61; update += 1) {
      await client.fetchQuery({ queryKey, queryFn, staleTime: 0 });
    }

    const query = client.getQueryCache().find<GitSourcePollingState[]>({ queryKey });
    expect(query?.state.dataUpdateCount).toBeGreaterThan(60);
    expect(gitSourcesRefetchInterval(query!)).toBe(GIT_ACTIVE_POLL_MS);
    client.clear();
  });

  it("polls indexed files only while the selected source is indexing", () => {
    expect(gitFilesRefetchInterval("indexing")).toBe(GIT_ACTIVE_POLL_MS);
    expect(gitFilesRefetchInterval("pending")).toBe(GIT_ACTIVE_POLL_MS);
    expect(gitFilesRefetchInterval("ready")).toBe(false);
    expect(gitFilesRefetchInterval("stale")).toBe(false);
    expect(gitFilesRefetchInterval("failed")).toBe(false);
  });
});
