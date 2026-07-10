import { computed, ref, watch, type ShallowUnwrapRef } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { useConfirm } from "primevue/useconfirm";

import {
  apiClient,
  type GroupMemberResponse,
  type GroupResponse,
  type UserDirectoryEntryResponse,
} from "../services/api";
import { setWorkspaceNavigationGroup } from "./use-workspace-navigation-context";

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
  const confirm = useConfirm();

  const groupPath = computed(() => String(route.params.groupPath ?? ""));
  const group = ref<GroupResponse | null>(null);
  const members = ref<GroupMemberResponse[]>([]);
  const childGroups = ref<GroupResponse[]>([]);
  const errorMessage = ref("");
  const memberError = ref("");
  const childGroupError = ref("");
  const loading = ref(false);

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

  async function loadPage() {
    loading.value = true;
    try {
      errorMessage.value = "";
      const [nextGroup, nextMembers, nextChildren, nextGroups] = await Promise.all([
        apiClient.getGroup(groupPath.value),
        apiClient.listGroupMembers(groupPath.value),
        apiClient.listChildGroups(groupPath.value),
        apiClient.listGroups(),
      ]);
      group.value = nextGroup;
      members.value = nextMembers;
      childGroups.value = nextChildren;
      groupSuggestions.value = nextGroups.filter((item: GroupResponse) => !isDescendantPath(item.group_path, nextGroup.group_path ?? groupPath.value));
      setWorkspaceNavigationGroup(nextGroup.group_path ?? groupPath.value, nextGroup.name);
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : t("groups.loadFailed");
      setWorkspaceNavigationGroup(groupPath.value);
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
      await apiClient.updateGroup(groupPath.value, {
        name: payload.name,
        visibility: payload.visibility,
      });
      groupDialogVisible.value = false;
      await loadPage();
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : t("groups.updateFailed");
    } finally {
      groupDialogBusy.value = false;
    }
  }

  async function saveChildGroup(payload: { key?: string; name: string; visibility: "private" | "public" }) {
    childGroupDialogBusy.value = true;
    childGroupError.value = "";
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
      childGroupError.value = error instanceof Error ? error.message : t("groups.childCreateFailed");
    } finally {
      childGroupDialogBusy.value = false;
    }
  }

  async function saveMember(payload: { login_name: string; role: "owner" | "maintainer" | "viewer" }) {
    memberDialogBusy.value = true;
    memberError.value = "";
    try {
      await apiClient.upsertGroupMember(groupPath.value, payload);
      memberDialogVisible.value = false;
      editingMember.value = null;
      selectedMemberUser.value = null;
      members.value = await apiClient.listGroupMembers(groupPath.value);
    } catch (error) {
      memberError.value = error instanceof Error ? error.message : t("groups.membersFailed");
    } finally {
      memberDialogBusy.value = false;
    }
  }

  async function submitMoveGroup() {
    if (!movingGroup.value?.group_path) return;
    childGroupDialogBusy.value = true;
    childGroupError.value = "";
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
          name: "group-detail",
          params: { groupPath: moved.group_path ?? moved.group_key },
        });
      }
    } catch (error) {
      childGroupError.value = error instanceof Error ? error.message : t("groups.moveFailed");
    } finally {
      childGroupDialogBusy.value = false;
    }
  }

  function openGroup(childGroup: GroupResponse) {
    void router.push({
      name: "group-detail",
      params: { groupPath: childGroup.group_path ?? childGroup.group_key },
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
    await apiClient.deleteGroup(groupPath.value);
    void router.push({ name: "groups" });
  }

  function confirmDeleteChildGroup(childGroup: GroupResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("groups.childDeleteConfirm", { name: childGroup.name }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => void deleteChildGroup(childGroup),
    });
  }

  async function deleteChildGroup(childGroup: GroupResponse) {
    if (!childGroup.group_path) return;
    await apiClient.deleteGroup(childGroup.group_path);
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
    await apiClient.deleteGroupMember(groupPath.value, loginName);
    members.value = await apiClient.listGroupMembers(groupPath.value);
  }

  function openCreateChildGroupDialog() {
    editingChildGroup.value = null;
    childGroupError.value = "";
    childGroupDialogVisible.value = true;
  }

  function openEditChildGroupDialog(childGroup: GroupResponse) {
    editingChildGroup.value = childGroup;
    childGroupError.value = "";
    childGroupDialogVisible.value = true;
  }

  function openMoveChildGroupDialog(childGroup: GroupResponse) {
    movingGroup.value = childGroup;
    selectedTargetGroup.value = null;
    childGroupError.value = "";
    moveGroupDialogVisible.value = true;
  }

  function openMoveCurrentGroupDialog() {
    if (!group.value) return;
    movingGroup.value = group.value;
    selectedTargetGroup.value = null;
    childGroupError.value = "";
    moveGroupDialogVisible.value = true;
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
    return option.group_path ? `${option.name} (${option.group_path})` : `${option.name} (${option.group_key})`;
  }

  watch(groupPath, (nextGroupPath) => {
    setWorkspaceNavigationGroup(nextGroupPath);
    void loadPage();
  }, { immediate: true });

  return {
    canManageGroup,
    canOwnGroup,
    childGroupDialogBusy,
    childGroupDialogVisible,
    childGroupError,
    childGroups,
    confirmDeleteChildGroup,
    confirmDeleteGroup,
    confirmRemoveMember,
    editingChildGroup,
    editingMember,
    errorMessage,
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
    memberError,
    memberSuggestions,
    members,
    moveGroupDialogVisible,
    movingGroup,
    openCreateChildGroupDialog,
    openCreateMemberDialog,
    openEditChildGroupDialog,
    openEditMemberDialog,
    openGroup,
    openMoveChildGroupDialog,
    openMoveCurrentGroupDialog,
    roleSeverity,
    saveChildGroup,
    saveGroup,
    saveMember,
    searchUsers,
    selectedMemberUser,
    selectedTargetGroup,
    submitMoveGroup,
  };
}

export type GroupWorkspaceState = ShallowUnwrapRef<ReturnType<typeof useGroupWorkspace>>;
