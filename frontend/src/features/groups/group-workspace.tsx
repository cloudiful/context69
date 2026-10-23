import { useQuery, useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type {
  CreateGroupRequest,
  GroupMemberResponse,
  GroupResponse,
  MemberSortBy,
  UpsertMembershipRequest,
  UserDirectoryEntryResponse,
  Visibility,
} from "@/services/api";

import { canManageGroup, canOwnGroup, groupPathOf, isDescendantPath } from "./permissions";

const DEFAULT_PAGE_SIZE = 50;

export interface GroupEntityDraft {
  key?: string;
  name: string;
  visibility: Visibility;
}

interface GroupWorkspaceValue {
  groupPath: string;
  group: GroupResponse | null;
  loading: boolean;
  error: unknown;
  canManage: boolean;
  canOwn: boolean;
  groupKey: string;

  children: GroupResponse[];
  childPage: { page: number; page_size: number; total: number; total_pages: number } | null;
  childrenLoading: boolean;
  childSearch: string;
  setChildSearch: (value: string) => void;
  setChildPage: (page: number) => void;

  members: GroupMemberResponse[];
  memberPage: { page: number; page_size: number; total: number; total_pages: number } | null;
  membersLoading: boolean;
  memberSearch: string;
  setMemberSearch: (value: string) => void;
  setMemberPage: (page: number) => void;
  setMemberSort: (field: MemberSortBy, direction: "asc" | "desc") => void;
  memberSort: { field: MemberSortBy; direction: "asc" | "desc" } | null;
  pageSize: number;
  setPageSize: (size: number) => void;

  editGroupOpen: boolean;
  setEditGroupOpen: (open: boolean) => void;
  groupBusy: boolean;
  saveGroup: (draft: GroupEntityDraft) => void;

  childDialogOpen: boolean;
  editingChild: GroupResponse | null;
  openCreateChild: () => void;
  openEditChild: (group: GroupResponse) => void;
  setChildDialogOpen: (open: boolean) => void;
  childBusy: boolean;
  saveChildGroup: (draft: GroupEntityDraft) => void;

  memberDialogOpen: boolean;
  editingMember: GroupMemberResponse | null;
  memberBusy: boolean;
  memberSuggestions: UserDirectoryEntryResponse[];
  searchMemberCandidates: (query: string) => void;
  openCreateMember: () => void;
  openEditMember: (member: GroupMemberResponse) => void;
  setMemberDialogOpen: (open: boolean) => void;
  saveMember: (payload: UpsertMembershipRequest) => void;

  moveDialogOpen: boolean;
  movingGroup: GroupResponse | null;
  moveBusy: boolean;
  moveSuggestions: GroupResponse[];
  openMoveGroup: (group: GroupResponse) => void;
  setMoveDialogOpen: (open: boolean) => void;
  searchMoveTargets: (query: string) => void;
  submitMove: (target: GroupResponse | null) => void;

  requestDeleteGroup: () => void;
  requestDeleteChild: (group: GroupResponse) => void;
  requestRemoveMember: (member: GroupMemberResponse) => void;

  confirm: { title: string; message: string; acceptLabel: string; action: () => void } | null;
  cancelConfirm: () => void;
  acceptConfirm: () => void;
}

const GroupWorkspaceContext = createContext<GroupWorkspaceValue | null>(null);

export function useGroupWorkspaceContext(): GroupWorkspaceValue {
  const value = useContext(GroupWorkspaceContext);
  if (!value) {
    throw new Error("useGroupWorkspaceContext requires a GroupWorkspaceProvider");
  }
  return value;
}

export function GroupWorkspaceProvider({
  groupPath,
  children,
}: {
  groupPath: string;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const groupQuery = useQuery({
    queryKey: ["group", groupPath],
    queryFn: ({ signal }) => apiClient.getGroup(groupPath, { signal }),
    retry: false,
  });

  const [childSearch, setChildSearchState] = useState("");
  const [childPage, setChildPageState] = useState(1);
  const [memberSearch, setMemberSearchState] = useState("");
  const [memberPage, setMemberPageState] = useState(1);
  const [memberSort, setMemberSort] = useState<{ field: MemberSortBy; direction: "asc" | "desc" } | null>(null);
  const [pageSize, setPageSizeState] = useState(DEFAULT_PAGE_SIZE);
  const [editGroupOpen, setEditGroupOpen] = useState(false);
  const [groupBusy, setGroupBusy] = useState(false);
  const [childDialogOpen, setChildDialogOpen] = useState(false);
  const [editingChild, setEditingChild] = useState<GroupResponse | null>(null);
  const [childBusy, setChildBusy] = useState(false);
  const [memberDialogOpen, setMemberDialogOpen] = useState(false);
  const [editingMember, setEditingMember] = useState<GroupMemberResponse | null>(null);
  const [memberBusy, setMemberBusy] = useState(false);
  const [memberSuggestions, setMemberSuggestions] = useState<UserDirectoryEntryResponse[]>([]);
  const [moveDialogOpen, setMoveDialogOpen] = useState(false);
  const [movingGroup, setMovingGroup] = useState<GroupResponse | null>(null);
  const [moveBusy, setMoveBusy] = useState(false);
  const [moveSuggestions, setMoveSuggestions] = useState<GroupResponse[]>([]);
  const [confirm, setConfirm] = useState<GroupWorkspaceValue["confirm"]>(null);

  useEffect(() => {
    setChildSearchState("");
    setChildPageState(1);
    setMemberSearchState("");
    setMemberPageState(1);
    setMemberSort(null);
    setEditGroupOpen(false);
    setChildDialogOpen(false);
    setEditingChild(null);
    setMemberDialogOpen(false);
    setEditingMember(null);
    setMoveDialogOpen(false);
    setMovingGroup(null);
  }, [groupPath]);

  const childrenQuery = useQuery({
    queryKey: ["group-children", groupPath, childPage, pageSize, childSearch],
    queryFn: ({ signal }) => apiClient.listChildGroups(groupPath, {
      page: childPage,
      page_size: pageSize,
      query: childSearch.trim() || undefined,
    }, { signal }),
    enabled: !!groupQuery.data,
    retry: false,
  });

  const membersQuery = useQuery({
    queryKey: [
      "group-members",
      groupPath,
      memberPage,
      pageSize,
      memberSearch,
      memberSort?.field ?? "",
      memberSort?.direction ?? "",
    ],
    queryFn: ({ signal }) => apiClient.listGroupMembers(groupPath, {
      page: memberPage,
      page_size: pageSize,
      query: memberSearch.trim() || undefined,
      sort_by: memberSort?.field,
      sort_direction: memberSort?.direction,
    }, { signal }),
    enabled: !!groupQuery.data,
    retry: false,
  });

  const group = groupQuery.data ?? null;

  const invalidateGroup = useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["group", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["group-children", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["group-members", groupPath] }),
    ]);
  }, [groupPath, queryClient]);

  const saveGroup = useCallback((draft: GroupEntityDraft) => {
    setGroupBusy(true);
    void (async () => {
      try {
        await apiClient.updateGroup(groupPath, {
          name: draft.name.trim(),
          visibility: draft.visibility,
        });
        setEditGroupOpen(false);
        await invalidateGroup();
        toast.success(t("common.save"), { duration: 2500 });
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("groups.updateFailed"),
          duration: 5000,
        });
      } finally {
        setGroupBusy(false);
      }
    })();
  }, [groupPath, invalidateGroup, t]);

  const saveChildGroup = useCallback((draft: GroupEntityDraft) => {
    setChildBusy(true);
    void (async () => {
      try {
        if (editingChild?.group_path) {
          await apiClient.updateGroup(editingChild.group_path, {
            name: draft.name.trim(),
            visibility: draft.visibility,
          });
        } else {
          const payload: CreateGroupRequest = {
            parent_group_path: groupPath,
            group_key: draft.key?.trim() ?? "",
            name: draft.name.trim(),
            visibility: draft.visibility,
            kind: "shared",
          };
          await apiClient.createGroup(payload);
        }
        setChildDialogOpen(false);
        setEditingChild(null);
        await invalidateGroup();
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("groups.childCreateFailed"),
          duration: 5000,
        });
      } finally {
        setChildBusy(false);
      }
    })();
  }, [editingChild, groupPath, invalidateGroup, t]);

  const saveMember = useCallback((payload: UpsertMembershipRequest) => {
    setMemberBusy(true);
    void (async () => {
      try {
        await apiClient.upsertGroupMember(groupPath, {
          login_name: payload.login_name.trim(),
          role: payload.role,
        });
        setMemberDialogOpen(false);
        setEditingMember(null);
        await invalidateGroup();
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("groups.membersFailed"),
          duration: 5000,
        });
      } finally {
        setMemberBusy(false);
      }
    })();
  }, [groupPath, invalidateGroup, t]);

  const searchMemberCandidates = useCallback((query: string) => {
    void (async () => {
      try {
        setMemberSuggestions(await apiClient.searchUserDirectory(query, 10));
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("adminUsers.loadFailed"),
          duration: 5000,
        });
      }
    })();
  }, [t]);

  const searchMoveTargets = useCallback((query: string) => {
    const movingPath = movingGroup ? groupPathOf(movingGroup) : groupPath;
    void (async () => {
      try {
        const results = await apiClient.searchGroups(query, 20);
        setMoveSuggestions(results.filter((candidate) => !isDescendantPath(groupPathOf(candidate), movingPath)));
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("groups.loadFailed"),
          duration: 5000,
        });
      }
    })();
  }, [groupPath, movingGroup, t]);

  const submitMove = useCallback((target: GroupResponse | null) => {
    if (!movingGroup) {
      return;
    }
    setMoveBusy(true);
    void (async () => {
      try {
        const moved = await apiClient.moveGroup(groupPathOf(movingGroup), {
          target_parent_group_path: target ? groupPathOf(target) : null,
        });
        setMoveDialogOpen(false);
        setMovingGroup(null);
        await invalidateGroup();
        if (groupPathOf(movingGroup) === groupPath) {
          navigate(`/groups/${encodeURIComponent(groupPathOf(moved))}`, { replace: true });
        }
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("groups.moveFailed"),
          duration: 5000,
        });
      } finally {
        setMoveBusy(false);
      }
    })();
  }, [groupPath, invalidateGroup, movingGroup, navigate, t]);

  const requestDeleteGroup = useCallback(() => {
    if (!group) {
      return;
    }
    setConfirm({
      title: t("common.delete"),
      message: t("groups.deleteConfirm", { name: group.name }),
      acceptLabel: t("common.delete"),
      action: () => {
        setConfirm(null);
        void (async () => {
          try {
            await apiClient.deleteGroup(groupPath);
            navigate("/groups", { replace: true });
          } catch (error) {
            toast.error(t("common.error"), {
              description: error instanceof Error ? error.message : t("groups.deleteFailed"),
              duration: 5000,
            });
          }
        })();
      },
    });
  }, [group, groupPath, navigate, t]);

  const requestDeleteChild = useCallback((child: GroupResponse) => {
    setConfirm({
      title: t("common.delete"),
      message: t("groups.childDeleteConfirm", { name: child.name }),
      acceptLabel: t("common.delete"),
      action: () => {
        setConfirm(null);
        void (async () => {
          try {
            await apiClient.deleteGroup(groupPathOf(child));
            await invalidateGroup();
          } catch (error) {
            toast.error(t("common.error"), {
              description: error instanceof Error ? error.message : t("groups.deleteFailed"),
              duration: 5000,
            });
          }
        })();
      },
    });
  }, [invalidateGroup, t]);

  const requestRemoveMember = useCallback((member: GroupMemberResponse) => {
    setConfirm({
      title: t("common.delete"),
      message: t("groups.memberRemoveConfirm", { loginName: member.login_name }),
      acceptLabel: t("common.delete"),
      action: () => {
        setConfirm(null);
        void (async () => {
          try {
            await apiClient.deleteGroupMember(groupPath, member.login_name);
            await invalidateGroup();
          } catch (error) {
            toast.error(t("common.error"), {
              description: error instanceof Error ? error.message : t("groups.membersFailed"),
              duration: 5000,
            });
          }
        })();
      },
    });
  }, [groupPath, invalidateGroup, t]);

  const openCreateChild = useCallback(() => {
    setEditingChild(null);
    setChildDialogOpen(true);
  }, []);

  const openEditChild = useCallback((child: GroupResponse) => {
    setEditingChild(child);
    setChildDialogOpen(true);
  }, []);

  const openCreateMember = useCallback(() => {
    setEditingMember(null);
    setMemberSuggestions([]);
    setMemberDialogOpen(true);
  }, []);

  const openEditMember = useCallback((member: GroupMemberResponse) => {
    setEditingMember(member);
    setMemberDialogOpen(true);
  }, []);

  const openMoveGroup = useCallback((target: GroupResponse) => {
    setMovingGroup(target);
    setMoveDialogOpen(true);
    setMoveSuggestions([]);
    void apiClient.searchGroups("", 20).then((results) => {
      setMoveSuggestions(results.filter((candidate) => !isDescendantPath(groupPathOf(candidate), groupPathOf(target))));
    }).catch(() => {
      setMoveSuggestions([]);
    });
  }, []);

  const value = useMemo<GroupWorkspaceValue>(() => ({
    groupPath,
    group,
    loading: groupQuery.isLoading,
    error: groupQuery.error,
    canManage: canManageGroup(group?.current_role),
    canOwn: canOwnGroup(group?.current_role),
    groupKey: group?.group_key ?? groupPath.split("/").filter(Boolean).at(-1) ?? "",

    children: childrenQuery.data?.items ?? [],
    childPage: childrenQuery.data?.pagination ?? null,
    childrenLoading: childrenQuery.isLoading,
    childSearch,
    setChildSearch: (next) => {
      setChildSearchState(next);
      setChildPageState(1);
    },
    setChildPage: setChildPageState,

    members: membersQuery.data?.items ?? [],
    memberPage: membersQuery.data?.pagination ?? null,
    membersLoading: membersQuery.isLoading,
    memberSearch,
    setMemberSearch: (next) => {
      setMemberSearchState(next);
      setMemberPageState(1);
    },
    setMemberPage: setMemberPageState,
    setMemberSort: (field, direction) => {
      setMemberSort({ field, direction });
      setMemberPageState(1);
    },
    memberSort,
    pageSize,
    setPageSize: (size) => {
      setPageSizeState(size);
      setChildPageState(1);
      setMemberPageState(1);
    },

    editGroupOpen,
    setEditGroupOpen,
    groupBusy,
    saveGroup,

    childDialogOpen,
    editingChild,
    openCreateChild,
    openEditChild,
    setChildDialogOpen,
    childBusy,
    saveChildGroup,

    memberDialogOpen,
    editingMember,
    memberBusy,
    memberSuggestions,
    searchMemberCandidates,
    openCreateMember,
    openEditMember,
    setMemberDialogOpen,
    saveMember,

    moveDialogOpen,
    movingGroup,
    moveBusy,
    moveSuggestions,
    openMoveGroup,
    setMoveDialogOpen,
    searchMoveTargets,
    submitMove,

    requestDeleteGroup,
    requestDeleteChild,
    requestRemoveMember,

    confirm,
    cancelConfirm: () => setConfirm(null),
    acceptConfirm: () => confirm?.action(),
  }), [
    group,
    childBusy,
    childDialogOpen,
    childPage,
    childSearch,
    childrenQuery.data,
    childrenQuery.isLoading,
    confirm,
    editGroupOpen,
    editingChild,
    editingMember,
    group,
    groupBusy,
    groupPath,
    groupQuery.error,
    groupQuery.isLoading,
    memberBusy,
    memberDialogOpen,
    memberPage,
    memberSearch,
    memberSort,
    memberSuggestions,
    membersQuery.data,
    membersQuery.isLoading,
    moveBusy,
    moveDialogOpen,
    moveSuggestions,
    movingGroup,
    openCreateChild,
    openCreateMember,
    openEditChild,
    openEditMember,
    openMoveGroup,
    pageSize,
    requestDeleteChild,
    requestDeleteGroup,
    requestRemoveMember,
    saveMember,
    saveChildGroup,
    saveGroup,
    searchMemberCandidates,
    searchMoveTargets,
    submitMove,
  ]);

  return (
    <GroupWorkspaceContext.Provider value={value}>
      {children}
    </GroupWorkspaceContext.Provider>
  );
}
