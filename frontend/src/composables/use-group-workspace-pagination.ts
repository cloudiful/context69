import { ref, watch, type Ref } from "vue";
import { useErrorToast } from "./use-error-toast";

import {
  apiClient,
  type GroupMemberPageResponse,
  type GroupMemberResponse,
  type GroupPageResponse,
  type GroupResponse,
  type MemberPageQuery,
  type MemberSortBy,
  type NamespacePageQuery,
} from "../services/api";

function emptyGroupPage(): GroupPageResponse {
  return { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } };
}

function emptyMemberPage(): GroupMemberPageResponse {
  return { items: [], pagination: { page: 1, page_size: 50, total: 0, total_pages: 0 } };
}

type Options = {
  groupPath: Ref<string>;
  t: (key: string) => string;
};

export function useGroupWorkspacePagination({ groupPath, t }: Options) {
  const showErrorToast = useErrorToast();
  const members = ref<GroupMemberResponse[]>([]);
  const childGroups = ref<GroupResponse[]>([]);
  const membersPage = ref<GroupMemberPageResponse>(emptyMemberPage());
  const childrenPage = ref<GroupPageResponse>(emptyGroupPage());
  const membersPageNumber = ref(1);
  const childrenPageNumber = ref(1);
  const pageSize = ref(50);
  const membersSearch = ref("");
  const childrenSearch = ref("");
  const membersSort = ref<{ field: MemberSortBy; direction: "asc" | "desc" } | null>(null);

  function pageQuery(page: number, query: string): NamespacePageQuery {
    return { page, page_size: pageSize.value, query: query.trim() || undefined };
  }

  function membersPageQuery(page: number, query: string): MemberPageQuery {
    return {
      page,
      page_size: pageSize.value,
      query: query.trim() || undefined,
      sort_by: membersSort.value?.field,
      sort_direction: membersSort.value?.direction,
    };
  }

  async function loadChildrenPage() {
    const response = await apiClient.listChildGroups(
      groupPath.value,
      pageQuery(childrenPageNumber.value, childrenSearch.value),
    );
    childrenPage.value = response;
    childGroups.value = response.items;
  }

  async function loadMembersPage() {
    const response = await apiClient.listGroupMembers(
      groupPath.value,
      membersPageQuery(membersPageNumber.value, membersSearch.value),
    );
    membersPage.value = response;
    members.value = response.items;
  }

  function reset() {
    childrenPageNumber.value = 1;
    membersPageNumber.value = 1;
    childrenSearch.value = "";
    membersSearch.value = "";
    membersSort.value = null;
    childrenPage.value = emptyGroupPage();
    membersPage.value = emptyMemberPage();
    childGroups.value = [];
    members.value = [];
  }

  function changeChildrenPage(page: number) {
    childrenPageNumber.value = page;
    void loadChildrenPage().catch((error) => showErrorToast(error, t("groups.loadFailed")));
  }

  function changeMembersPage(page: number) {
    membersPageNumber.value = page;
    void loadMembersPage().catch((error) => showErrorToast(error, t("groups.membersFailed")));
  }

  function changeMembersSort(field: MemberSortBy, direction: "asc" | "desc") {
    if (membersSort.value?.field === field && membersSort.value?.direction === direction) return;
    membersSort.value = { field, direction };
    membersPageNumber.value = 1;
    void loadMembersPage().catch((error) => showErrorToast(error, t("groups.membersFailed")));
  }

  function clearMembersSort() {
    if (!membersSort.value) return;
    membersSort.value = null;
    membersPageNumber.value = 1;
    void loadMembersPage().catch((error) => showErrorToast(error, t("groups.membersFailed")));
  }

  function changePageSize(value: number) {
    if (pageSize.value === value) return;
    pageSize.value = value;
    childrenPageNumber.value = 1;
    membersPageNumber.value = 1;
    void Promise.all([loadChildrenPage(), loadMembersPage()]).catch((error) => showErrorToast(error, t("groups.loadFailed")));
  }

  watch(childrenSearch, () => {
    childrenPageNumber.value = 1;
    void loadChildrenPage().catch((error) => showErrorToast(error, t("groups.loadFailed")));
  });

  watch(membersSearch, () => {
    membersPageNumber.value = 1;
    void loadMembersPage().catch((error) => showErrorToast(error, t("groups.membersFailed")));
  });

  return {
    changeChildrenPage,
    changeMembersPage,
    changeMembersSort,
    clearMembersSort,
    changePageSize,
    childGroups,
    childrenPage,
    childrenPageNumber,
    childrenSearch,
    loadChildrenPage,
    loadMembersPage,
    members,
    membersPage,
    membersPageNumber,
    membersSearch,
    membersSort,
    pageSize,
    reset,
  };
}
