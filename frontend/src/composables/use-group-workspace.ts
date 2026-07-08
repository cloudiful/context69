import { computed, ref, watch, type ShallowUnwrapRef } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { useConfirm } from "primevue/useconfirm";

import {
  apiClient,
  type GroupMemberResponse,
  type GroupResponse,
  type ProjectResponse,
  type UserDirectoryEntryResponse,
} from "../services/api";
import { setWorkspaceNavigationGroup } from "./use-workspace-navigation-context";

export function useGroupWorkspace() {
  const route = useRoute();
  const router = useRouter();
  const { t } = useI18n();
  const confirm = useConfirm();

  const groupKey = computed(() => String(route.params.groupKey ?? ""));
  const group = ref<GroupResponse | null>(null);
  const members = ref<GroupMemberResponse[]>([]);
  const projects = ref<ProjectResponse[]>([]);
  const errorMessage = ref("");
  const memberError = ref("");
  const projectError = ref("");
  const loading = ref(false);

  const groupDialogVisible = ref(false);
  const groupDialogBusy = ref(false);
  const projectDialogVisible = ref(false);
  const projectDialogBusy = ref(false);
  const moveProjectDialogVisible = ref(false);
  const memberDialogVisible = ref(false);
  const memberDialogBusy = ref(false);

  const editingProject = ref<ProjectResponse | null>(null);
  const movingProject = ref<ProjectResponse | null>(null);
  const editingMember = ref<GroupMemberResponse | null>(null);
  const memberSuggestions = ref<UserDirectoryEntryResponse[]>([]);
  const selectedMemberUser = ref<UserDirectoryEntryResponse | null>(null);
  const groupSuggestions = ref<GroupResponse[]>([]);
  const selectedTargetGroup = ref<GroupResponse | null>(null);

  async function loadPage() {
    loading.value = true;
    try {
      errorMessage.value = "";
      const [nextGroup, nextMembers, nextProjects, nextGroups] = await Promise.all([
        apiClient.getGroup(groupKey.value),
        apiClient.listGroupMembers(groupKey.value),
        apiClient.listProjects(groupKey.value),
        apiClient.listGroups(),
      ]);
      group.value = nextGroup;
      members.value = nextMembers;
      projects.value = nextProjects;
      groupSuggestions.value = nextGroups.filter((item) => item.group_key !== groupKey.value);
      setWorkspaceNavigationGroup(groupKey.value, nextGroup.name);
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : t("groups.loadFailed");
      setWorkspaceNavigationGroup(groupKey.value);
    } finally {
      loading.value = false;
    }
  }

  async function searchUsers(query: string) {
    memberSuggestions.value = await apiClient.searchUserDirectory(query, 10);
  }

  async function saveGroup(payload: { name: string; visibility: "private" | "public" }) {
    groupDialogBusy.value = true;
    errorMessage.value = "";
    try {
      await apiClient.updateGroup(groupKey.value, {
        name: payload.name,
        visibility: payload.visibility,
      });
      groupDialogVisible.value = false;
      await loadPage();
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : t("groups.createFailed");
    } finally {
      groupDialogBusy.value = false;
    }
  }

  async function saveProject(payload: { key?: string; name: string; visibility: "private" | "public" }) {
    projectDialogBusy.value = true;
    projectError.value = "";
    try {
      if (editingProject.value) {
        await apiClient.updateProject(groupKey.value, editingProject.value.project_key, {
          name: payload.name,
          visibility: payload.visibility,
        });
      } else {
        await apiClient.createProject(groupKey.value, {
          project_key: payload.key?.trim() ?? "",
          name: payload.name,
          visibility: payload.visibility,
        });
      }
      projectDialogVisible.value = false;
      editingProject.value = null;
      await loadPage();
    } catch (error) {
      projectError.value = error instanceof Error ? error.message : t("groups.projectCreateFailed");
    } finally {
      projectDialogBusy.value = false;
    }
  }

  async function saveMember(payload: { login_name: string; role: "owner" | "maintainer" | "viewer" }) {
    memberDialogBusy.value = true;
    memberError.value = "";
    try {
      await apiClient.upsertGroupMember(groupKey.value, payload);
      memberDialogVisible.value = false;
      editingMember.value = null;
      selectedMemberUser.value = null;
      members.value = await apiClient.listGroupMembers(groupKey.value);
    } catch (error) {
      memberError.value = error instanceof Error ? error.message : t("groups.membersFailed");
    } finally {
      memberDialogBusy.value = false;
    }
  }

  async function submitMoveProject() {
    if (!movingProject.value || !selectedTargetGroup.value) return;
    projectDialogBusy.value = true;
    projectError.value = "";
    try {
      const moved = await apiClient.moveProject(groupKey.value, movingProject.value.project_key, {
        target_group_key: selectedTargetGroup.value.group_key,
      });
      moveProjectDialogVisible.value = false;
      movingProject.value = null;
      selectedTargetGroup.value = null;
      await loadPage();
      void router.push({
        name: "project",
        params: {
          groupKey: moved.group_key,
          projectKey: moved.project_key,
        },
      });
    } catch (error) {
      projectError.value = error instanceof Error ? error.message : t("groups.projectMoveFailed");
    } finally {
      projectDialogBusy.value = false;
    }
  }

  function openProject(projectItem: ProjectResponse) {
    void router.push({
      name: "project",
      params: { groupKey: groupKey.value, projectKey: projectItem.project_key },
    });
  }

  function confirmDeleteGroup() {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.deleteConfirm", { name: group.value?.name ?? groupKey.value }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => void deleteGroup(),
    });
  }

  async function deleteGroup() {
    await apiClient.deleteGroup(groupKey.value);
    void router.push({ name: "groups" });
  }

  function confirmDeleteProject(projectItem: ProjectResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.projectDeleteConfirm", { name: projectItem.name }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => void deleteProject(projectItem),
    });
  }

  async function deleteProject(projectItem: ProjectResponse) {
    await apiClient.deleteProject(groupKey.value, projectItem.project_key);
    await loadPage();
  }

  function confirmRemoveMember(member: GroupMemberResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.memberRemoveConfirm", { loginName: member.login_name }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => void removeMember(member.login_name),
    });
  }

  async function removeMember(loginName: string) {
    await apiClient.deleteGroupMember(groupKey.value, loginName);
    members.value = await apiClient.listGroupMembers(groupKey.value);
  }

  function openCreateProjectDialog() {
    editingProject.value = null;
    projectError.value = "";
    projectDialogVisible.value = true;
  }

  function openEditProjectDialog(projectItem: ProjectResponse) {
    editingProject.value = projectItem;
    projectError.value = "";
    projectDialogVisible.value = true;
  }

  function openMoveProjectDialog(projectItem: ProjectResponse) {
    movingProject.value = projectItem;
    selectedTargetGroup.value = null;
    projectError.value = "";
    moveProjectDialogVisible.value = true;
  }

  function openCreateMemberDialog() {
    editingMember.value = null;
    selectedMemberUser.value = null;
    memberError.value = "";
    memberDialogVisible.value = true;
  }

  function openEditMemberDialog(member: GroupMemberResponse) {
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

  watch(groupKey, (nextGroupKey) => {
    setWorkspaceNavigationGroup(nextGroupKey);
    void loadPage();
  }, { immediate: true });

  return {
    groupKey,
    group,
    members,
    projects,
    errorMessage,
    memberError,
    projectError,
    loading,
    groupDialogVisible,
    groupDialogBusy,
    projectDialogVisible,
    projectDialogBusy,
    moveProjectDialogVisible,
    memberDialogVisible,
    memberDialogBusy,
    editingProject,
    movingProject,
    editingMember,
    memberSuggestions,
    selectedMemberUser,
    groupSuggestions,
    selectedTargetGroup,
    searchUsers,
    saveGroup,
    saveProject,
    saveMember,
    submitMoveProject,
    openProject,
    confirmDeleteGroup,
    confirmDeleteProject,
    confirmRemoveMember,
    openCreateProjectDialog,
    openEditProjectDialog,
    openMoveProjectDialog,
    openCreateMemberDialog,
    openEditMemberDialog,
    roleSeverity,
    groupOptionLabel,
  };
}

export type GroupWorkspaceState = ShallowUnwrapRef<ReturnType<typeof useGroupWorkspace>>;
