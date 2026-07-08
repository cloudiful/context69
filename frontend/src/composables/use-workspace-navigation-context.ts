import { shallowReactive } from "vue";

const state = shallowReactive({
  groupKey: "",
  groupLabel: "",
  projectGroupKey: "",
  projectKey: "",
  projectLabel: "",
});

export function useWorkspaceNavigationContext() {
  return state;
}

export function setWorkspaceNavigationGroup(groupKey: string, label = "") {
  state.groupKey = groupKey;
  state.groupLabel = label;
}

export function setWorkspaceNavigationProject(groupKey: string, projectKey: string, label = "") {
  state.projectGroupKey = groupKey;
  state.projectKey = projectKey;
  state.projectLabel = label;
}
