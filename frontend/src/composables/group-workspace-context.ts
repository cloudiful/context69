import { inject, type InjectionKey } from "vue";

import type { GroupWorkspaceState } from "./use-group-workspace";

export const groupWorkspaceStateKey: InjectionKey<GroupWorkspaceState> = Symbol("group-workspace-state");

export function useGroupWorkspaceContext(): GroupWorkspaceState {
  const state = inject(groupWorkspaceStateKey);
  if (!state) {
    throw new Error("group workspace state is not available");
  }

  return state;
}
