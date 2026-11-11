import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";

import { apiClient } from "@/services/api";
import type { GitRepositoryRegistrationRequest } from "@/services/api";

export function invalidateGitSources(queryClient: QueryClient, groupPath: string) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: ["git-sources", groupPath] }),
    queryClient.invalidateQueries({ queryKey: ["git-files", groupPath] }),
    queryClient.invalidateQueries({ queryKey: ["git-file", groupPath] }),
    queryClient.invalidateQueries({ queryKey: ["git-content", groupPath] }),
    queryClient.invalidateQueries({ queryKey: ["git-code-search", groupPath] }),
  ]);
}

export function useGitConnections(groupPath: string, options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ["git-connections", groupPath],
    queryFn: ({ signal }) => apiClient.listGitConnections(groupPath, { signal }),
    retry: false,
    enabled: options.enabled ?? true,
  });
}

export function useGitRepositoryRegistration(
  groupPath: string,
  handlers: {
    onSuccess?: () => void | Promise<void>;
    onError?: (error: unknown) => void;
  } = {},
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: GitRepositoryRegistrationRequest) => apiClient.registerGitRepository(groupPath, body),
    onSuccess: async () => {
      await invalidateGitSources(queryClient, groupPath);
      await handlers.onSuccess?.();
    },
    onError: (error) => handlers.onError?.(error),
    retry: false,
  });
}
