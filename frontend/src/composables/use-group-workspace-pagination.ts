import { ref, watch, type Ref } from "vue";
import { useErrorToast } from "./use-error-toast";

import {
  apiClient,
  type GroupMemberPageResponse,
  type GroupMemberResponse,
  type GroupPageResponse,
  type GroupResponse,
  type NamespacePageQuery,
} from "../services/api";

function emptyGroupPage(): GroupPageResponse {
  return { items: [], page: 1, page_size: 50, total: 0, total_pages: 0 };
}

function emptyMemberPage(): GroupMemberPageResponse {
  return { items: [], page: 1, page_size: 50, total: 0, total_pages: 0 };
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

  function pageQuery(page: number, query: string): NamespacePageQuery {
    return { page, page_size: pageSize.value, query: query.trim() || undefined };
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
      pageQuery(membersPageNumber.value, membersSearch.value),
    );
    membersPage.value = response;
    members.value = response.items;
  }

  function reset() {
    childrenPageNumber.value = 1;
    membersPageNumber.value = 1;
    childrenSearch.value = "";
    membersSearch.value = "";
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
    pageSize,
    reset,
  };
}
