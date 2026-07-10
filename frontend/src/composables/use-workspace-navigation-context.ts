import { shallowReactive } from "vue";

const state = shallowReactive({
  groupPath: "",
  groupLabel: "",
});

export function useWorkspaceNavigationContext() {
  return state;
}

export function setWorkspaceNavigationGroup(groupPath: string, label = "") {
  state.groupPath = groupPath;
  state.groupLabel = label;
}
