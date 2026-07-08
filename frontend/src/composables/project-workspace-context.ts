import { inject, type InjectionKey } from "vue";

import type { ProjectWorkspaceState } from "./use-project-workspace";

export const projectWorkspaceStateKey: InjectionKey<ProjectWorkspaceState> = Symbol("project-workspace-state");

export function useProjectWorkspaceContext(): ProjectWorkspaceState {
  const state = inject(projectWorkspaceStateKey);
  if (!state) {
    throw new Error("project workspace state is not available");
  }

  return state;
}
