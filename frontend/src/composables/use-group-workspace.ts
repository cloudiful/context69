import { computed, ref, watch, type ShallowUnwrapRef } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { useAppConfirm } from "./use-app-confirm";

import {
  apiClient,
  type CreateGroupRequest,
  type GroupMemberResponse,
  type GroupResponse,
  type UpsertMembershipRequest,
  type UserDirectoryEntryResponse,
} from "../services/api";
import { setWorkspaceNavigationGroup } from "./use-workspace-navigation-context";
import { useErrorToast } from "./use-error-toast";
import { useGroupWorkspacePagination } from "./use-group-workspace-pagination";

function roleRank(role?: string | null) {
  if (role === "owner") return 3;
  if (role === "maintainer") return 2;
  if (role === "viewer") return 1;
  return 0;
}

function isDescendantPath(candidatePath: string | undefined | null, parentPath: string) {
  return !!candidatePath && (candidatePath === parentPath || candidatePath.startsWith(`${parentPath}/`));
}

export function useGroupWorkspace() {
  const route = useRoute();
  const router = useRouter();
  const { t } = useI18n();
  const confirm = useAppConfirm();
  const showErrorToast = useErrorToast();

  const groupPath = computed(() => String(route.params.groupPath ?? ""));
  const group = ref<GroupResponse | null>(null);
  const loading = ref(false);
  const pagination = useGroupWorkspacePagination({ groupPath, t });
  const {
    changeChildrenPage, changeMembersPage, changePageSize, childGroups, childrenPage, childrenPageNumber,
    childrenSearch, loadChildrenPage, loadMembersPage, members, membersPage, membersPageNumber,
    membersSearch, pageSize, reset: resetPagination,
  } = pagination;

  const groupDialogVisible = ref(false);
  const groupDialogBusy = ref(false);
  const childGroupDialogVisible = ref(false);
  const childGroupDialogBusy = ref(false);
  const moveGroupDialogVisible = ref(false);
  const memberDialogVisible = ref(false);
  const memberDialogBusy = ref(false);

  const editingChildGroup = ref<GroupResponse | null>(null);
  const movingGroup = ref<GroupResponse | null>(null);
  const editingMember = ref<GroupMemberResponse | null>(null);
  const memberSuggestions = ref<UserDirectoryEntryResponse[]>([]);
  const selectedMemberUser = ref<UserDirectoryEntryResponse | null>(null);
  const groupSuggestions = ref<GroupResponse[]>([]);
  const selectedTargetGroup = ref<GroupResponse | null>(null);

  const groupKey = computed(() => group.value?.group_key ?? groupPath.value.split("/").filter(Boolean).at(-1) ?? "");
  const canManageGroup = computed(() => roleRank(group.value?.current_role) >= 2);
  const canOwnGroup = computed(() => roleRank(group.value?.current_role) >= 3);

  function filterGroupSuggestions(groups: GroupResponse[]) {
    const movingPath = movingGroup.value?.group_path ?? groupPath.value;
    return groups.filter((item) => !isDescendantPath(item.group_path, movingPath));
  }

  async function loadPage() {
    loading.value = true;
    try {
      const nextGroup = await apiClient.getGroup(groupPath.value);
      group.value = nextGroup;
      await Promise.all([loadMembersPage(), loadChildrenPage()]);
      groupSuggestions.value = filterGroupSuggestions(groupSuggestions.value);
      setWorkspaceNavigationGroup(nextGroup.group_path ?? groupPath.value, nextGroup.name);
    } catch (error) {
      showErrorToast(error, t("groups.loadFailed"));
      setWorkspaceNavigationGroup(groupPath.value);
    } finally {
      loading.value = false;
    }
  }

  async function searchUsers(query: string) {
    try {
      memberSuggestions.value = await apiClient.searchUserDirectory(query, 10);
    } catch (error) {
      showErrorToast(error, t("adminUsers.loadFailed"));
    }
  }

  async function searchGroupTargets(query: string) {
    try {
      groupSuggestions.value = filterGroupSuggestions(await apiClient.searchGroups(query, 20));
    } catch (error) {
      showErrorToast(error, t("groups.loadFailed"));
    }
  }

  async function saveGroup(payload: Pick<CreateGroupRequest, "name" | "visibility">) {
    groupDialogBusy.value = true;
    try {
      await apiClient.updateGroup(groupPath.value, {
        name: payload.name,
        visibility: payload.visibility,
      });
      groupDialogVisible.value = false;
      await loadPage();
    } catch (error) {
      showErrorToast(error, t("groups.updateFailed"));
    } finally {
      groupDialogBusy.value = false;
    }
  }

  async function saveChildGroup(payload: Pick<CreateGroupRequest, "name" | "visibility"> & { key?: string }) {
    childGroupDialogBusy.value = true;
    try {
      if (editingChildGroup.value?.group_path) {
        await apiClient.updateGroup(editingChildGroup.value.group_path, {
          name: payload.name,
          visibility: payload.visibility,
        });
      } else {
        await apiClient.createGroup({
          parent_group_path: groupPath.value,
          group_key: payload.key?.trim() ?? "",
          name: payload.name,
          visibility: payload.visibility,
          kind: "shared",
        });
      }
      childGroupDialogVisible.value = false;
      editingChildGroup.value = null;
      await loadPage();
    } catch (error) {
      showErrorToast(error, t("groups.childCreateFailed"));
    } finally {
      childGroupDialogBusy.value = false;
    }
  }

  async function saveMember(payload: UpsertMembershipRequest) {
    memberDialogBusy.value = true;
    try {
      await apiClient.upsertGroupMember(groupPath.value, payload);
      memberDialogVisible.value = false;
      editingMember.value = null;
      selectedMemberUser.value = null;
      await loadMembersPage();
    } catch (error) {
      showErrorToast(error, t("groups.membersFailed"));
    } finally {
      memberDialogBusy.value = false;
    }
  }

  async function submitMoveGroup() {
    if (!movingGroup.value?.group_path) return;
    childGroupDialogBusy.value = true;
    try {
      const moved = await apiClient.moveGroup(movingGroup.value.group_path, {
        target_parent_group_path: selectedTargetGroup.value?.group_path ?? null,
      });
      const isCurrentGroup = movingGroup.value.group_path === groupPath.value;
      moveGroupDialogVisible.value = false;
      movingGroup.value = null;
      selectedTargetGroup.value = null;
      await loadPage();
      if (isCurrentGroup) {
        void router.push({
          name: "group-overview",
          params: { groupPath: moved.group_path ?? moved.group_key },
        });
      }
    } catch (error) {
      showErrorToast(error, t("groups.moveFailed"));
    } finally {
      childGroupDialogBusy.value = false;
    }
  }

  function openGroup(childGroup: GroupResponse) {
    void router.push({
      name: "group-overview",
      params: { groupPath: childGroup.group_path ?? childGroup.group_key },
    });
  }

  function confirmDeleteGroup() {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.deleteConfirm", { name: group.value?.name ?? groupKey.value }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("common.delete"),
      accept: () => void deleteGroup(),
    });
  }

  async function deleteGroup() {
    try {
      await apiClient.deleteGroup(groupPath.value);
      void router.push({ name: "groups" });
    } catch (error) {
      showErrorToast(error, t("groups.deleteFailed"));
    }
  }

  function confirmDeleteChildGroup(childGroup: GroupResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.childDeleteConfirm", { name: childGroup.name }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("common.delete"),
      accept: () => void deleteChildGroup(childGroup),
    });
  }

  async function deleteChildGroup(childGroup: GroupResponse) {
    if (!childGroup.group_path) return;
    try {
      await apiClient.deleteGroup(childGroup.group_path);
      await loadPage();
    } catch (error) {
      showErrorToast(error, t("groups.deleteFailed"));
    }
  }

  function confirmRemoveMember(member: GroupMemberResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.memberRemoveConfirm", { loginName: member.login_name }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("common.delete"),
      accept: () => void removeMember(member.login_name),
    });
  }

  async function removeMember(loginName: string) {
    try {
      await apiClient.deleteGroupMember(groupPath.value, loginName);
      await loadMembersPage();
    } catch (error) {
      showErrorToast(error, t("groups.membersFailed"));
    }
  }

  function openCreateChildGroupDialog() {
    editingChildGroup.value = null;
    childGroupDialogVisible.value = true;
  }

  function openEditChildGroupDialog(childGroup: GroupResponse) {
    editingChildGroup.value = childGroup;
    childGroupDialogVisible.value = true;
  }

  function openMoveChildGroupDialog(childGroup: GroupResponse) {
    movingGroup.value = childGroup;
    selectedTargetGroup.value = null;
    moveGroupDialogVisible.value = true;
    void searchGroupTargets("");
  }

  function openMoveCurrentGroupDialog() {
    if (!group.value) return;
    movingGroup.value = group.value;
    selectedTargetGroup.value = null;
    moveGroupDialogVisible.value = true;
    void searchGroupTargets("");
  }

  function openCreateMemberDialog() {
    editingMember.value = null;
    selectedMemberUser.value = null;
    memberDialogVisible.value = true;
  }

  function openEditMemberDialog(member: GroupMemberResponse) {
    editingMember.value = member;
    selectedMemberUser.value = {
      user_id: member.user_id,
      login_name: member.login_name,
      display_name: member.display_name,
    };
    memberDialogVisible.value = true;
  }

  function roleSeverity(role?: string | null) {
    if (role === "owner") return "success";
    if (role === "maintainer") return "info";
    return "secondary";
  }

  function groupOptionLabel(option: GroupResponse) {
    return option.group_path ? `${option.name} (${option.group_path})` : `${option.name} (${option.group_key})`;
  }

  watch(groupPath, (nextGroupPath) => {
    resetPagination();
    groupSuggestions.value = [];
    setWorkspaceNavigationGroup(nextGroupPath);
    void loadPage();
  }, { immediate: true });

  return {
    canManageGroup,
    canOwnGroup,
    changeChildrenPage,
    changeMembersPage,
    changePageSize,
    childGroupDialogBusy,
    childGroupDialogVisible,
    childGroups,
    childrenPage,
    childrenPageNumber,
    childrenSearch,
    confirmDeleteChildGroup,
    confirmDeleteGroup,
    confirmRemoveMember,
    editingChildGroup,
    editingMember,
    group,
    groupDialogBusy,
    groupDialogVisible,
    groupKey,
    groupPath,
    groupSuggestions,
    groupOptionLabel,
    loading,
    memberDialogBusy,
    memberDialogVisible,
    memberSuggestions,
    members,
    membersPage,
    membersPageNumber,
    membersSearch,
    moveGroupDialogVisible,
    movingGroup,
    openCreateChildGroupDialog,
    openCreateMemberDialog,
    openEditChildGroupDialog,
    openEditMemberDialog,
    openGroup,
    openMoveChildGroupDialog,
    openMoveCurrentGroupDialog,
    pageSize,
    roleSeverity,
    saveChildGroup,
    saveGroup,
    saveMember,
    searchUsers,
    searchGroupTargets,
    selectedMemberUser,
    selectedTargetGroup,
    submitMoveGroup,
  };
}

export type GroupWorkspaceState = ShallowUnwrapRef<ReturnType<typeof useGroupWorkspace>>;
