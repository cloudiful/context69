import { computed, ref, watch, type ShallowUnwrapRef } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { useConfirm } from "primevue/useconfirm";

import {
  apiClient,
  type GroupResponse,
  type ProjectMemberResponse,
  type ProjectResponse,
  type UserDirectoryEntryResponse,
} from "../services/api";
import { setWorkspaceNavigationGroup, setWorkspaceNavigationProject } from "./use-workspace-navigation-context";

export function useProjectWorkspace() {
  const route = useRoute();
  const router = useRouter();
  const { t } = useI18n();
  const confirm = useConfirm();

  const groupKey = computed(() => String(route.params.groupKey ?? ""));
  const projectKey = computed(() => String(route.params.projectKey ?? ""));
  const project = ref<ProjectResponse | null>(null);
  const members = ref<ProjectMemberResponse[]>([]);
  const groups = ref<GroupResponse[]>([]);
  const errorMessage = ref("");
  const memberError = ref("");
  const actionError = ref("");

  const projectDialogVisible = ref(false);
  const moveDialogVisible = ref(false);
  const memberDialogVisible = ref(false);
  const actionBusy = ref(false);
  const editingMember = ref<ProjectMemberResponse | null>(null);
  const memberSuggestions = ref<UserDirectoryEntryResponse[]>([]);
  const selectedMemberUser = ref<UserDirectoryEntryResponse | null>(null);
  const selectedTargetGroup = ref<GroupResponse | null>(null);

  function roleRank(role?: string | null) {
    if (role === "owner") return 3;
    if (role === "maintainer") return 2;
    if (role === "viewer") return 1;
    return 0;
  }

  const canManageProject = computed(() => roleRank(project.value?.current_role) >= 2);
  const canOwnProject = computed(() => roleRank(project.value?.current_role) >= 3);

  async function loadProject() {
    try {
      errorMessage.value = "";
      const [nextProject, nextMembers, nextGroups] = await Promise.all([
        apiClient.getProject(groupKey.value, projectKey.value),
        apiClient.listProjectMembers(groupKey.value, projectKey.value),
        apiClient.listGroups(),
      ]);
      project.value = nextProject;
      members.value = nextMembers;
      groups.value = nextGroups.filter((item) => item.group_key !== groupKey.value);
      setWorkspaceNavigationGroup(groupKey.value);
      setWorkspaceNavigationProject(groupKey.value, projectKey.value, nextProject.name);
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : t("project.loadFailed");
      setWorkspaceNavigationGroup(groupKey.value);
      setWorkspaceNavigationProject(groupKey.value, projectKey.value);
    }
  }

  async function searchUsers(query: string) {
    memberSuggestions.value = await apiClient.searchUserDirectory(query, 10);
  }

  async function saveProject(payload: { name: string; visibility: "private" | "public" }) {
    actionBusy.value = true;
    actionError.value = "";
    try {
      await apiClient.updateProject(groupKey.value, projectKey.value, {
        name: payload.name,
        visibility: payload.visibility,
      });
      projectDialogVisible.value = false;
      await loadProject();
    } catch (error) {
      actionError.value = error instanceof Error ? error.message : t("project.updateFailed");
    } finally {
      actionBusy.value = false;
    }
  }

  async function moveProject() {
    if (!selectedTargetGroup.value) return;
    actionBusy.value = true;
    actionError.value = "";
    try {
      const moved = await apiClient.moveProject(groupKey.value, projectKey.value, {
        target_group_key: selectedTargetGroup.value.group_key,
      });
      moveDialogVisible.value = false;
      selectedTargetGroup.value = null;
      void router.push({
        name: "project",
        params: { groupKey: moved.group_key, projectKey: moved.project_key },
      });
    } catch (error) {
      actionError.value = error instanceof Error ? error.message : t("project.moveFailed");
    } finally {
      actionBusy.value = false;
    }
  }

  async function saveMember(payload: { login_name: string; role: "owner" | "maintainer" | "viewer" }) {
    actionBusy.value = true;
    memberError.value = "";
    try {
      await apiClient.upsertProjectMember(groupKey.value, projectKey.value, payload);
      memberDialogVisible.value = false;
      editingMember.value = null;
      selectedMemberUser.value = null;
      members.value = await apiClient.listProjectMembers(groupKey.value, projectKey.value);
    } catch (error) {
      memberError.value = error instanceof Error ? error.message : t("project.membersFailed");
    } finally {
      actionBusy.value = false;
    }
  }

  function confirmDeleteProject() {
    confirm.require({
      header: t("common.delete"),
      message: t("project.deleteConfirm", { name: project.value?.name ?? projectKey.value }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => void deleteProject(),
    });
  }

  async function deleteProject() {
    await apiClient.deleteProject(groupKey.value, projectKey.value);
    void router.push({ name: "group-detail", params: { groupKey: groupKey.value } });
  }

  function confirmRemoveMember(member: ProjectMemberResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("project.memberRemoveConfirm", { loginName: member.login_name }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => void removeMember(member.login_name),
    });
  }

  async function removeMember(loginName: string) {
    await apiClient.deleteProjectMember(groupKey.value, projectKey.value, loginName);
    members.value = await apiClient.listProjectMembers(groupKey.value, projectKey.value);
  }

  function openCreateMemberDialog() {
    editingMember.value = null;
    selectedMemberUser.value = null;
    memberError.value = "";
    memberDialogVisible.value = true;
  }

  function openEditMemberDialog(member: ProjectMemberResponse) {
    editingMember.value = member;
    selectedMemberUser.value = {
      user_id: member.user_id,
      login_name: member.login_name,
      display_name: member.display_name,
    };
    memberError.value = "";
    memberDialogVisible.value = true;
  }

  function roleSeverity(role?: string | null) {
    if (role === "owner") return "success";
    if (role === "maintainer") return "info";
    return "secondary";
  }

  function groupOptionLabel(option: GroupResponse) {
    return `${option.name} (${option.group_key})`;
  }

  watch([groupKey, projectKey], ([nextGroupKey, nextProjectKey]) => {
    setWorkspaceNavigationGroup(nextGroupKey);
    setWorkspaceNavigationProject(nextGroupKey, nextProjectKey);
    void loadProject();
  }, { immediate: true });

  return {
    groupKey,
    projectKey,
    project,
    members,
    groups,
    errorMessage,
    memberError,
    actionError,
    projectDialogVisible,
    moveDialogVisible,
    memberDialogVisible,
    actionBusy,
    editingMember,
    memberSuggestions,
    selectedMemberUser,
    selectedTargetGroup,
    canManageProject,
    canOwnProject,
    searchUsers,
    saveProject,
    moveProject,
    saveMember,
    confirmDeleteProject,
    confirmRemoveMember,
    openCreateMemberDialog,
    openEditMemberDialog,
    roleSeverity,
    groupOptionLabel,
  };
}

export type ProjectWorkspaceState = ShallowUnwrapRef<ReturnType<typeof useProjectWorkspace>>;
