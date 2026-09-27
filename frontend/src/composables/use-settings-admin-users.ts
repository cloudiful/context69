import { ref } from "vue";
import { useI18n } from "vue-i18n";

import {
  apiClient,
  type AdminUserResponse,
  type AdminUserPageResponse,
  type AdminUserSortBy,
} from "../services/api";
import { authSessionState } from "../services/auth/session";
import { useErrorToast } from "./use-error-toast";
import { emptyPagination } from "./use-server-pagination";

type AdminUserSort = {
  field: AdminUserSortBy;
  direction: "asc" | "desc";
};

export function useSettingsAdminUsers() {
  const { t } = useI18n();
  const showErrorToast = useErrorToast();

  const adminUsers = ref<AdminUserResponse[]>([]);
  const adminUsersPage = ref<AdminUserPageResponse>({ items: [], pagination: emptyPagination(50) });
  const adminUsersPageNumber = ref(1);
  const adminUsersPageSize = ref(50);
  const adminUsersQuery = ref("");
  const adminUsersSort = ref<AdminUserSort | null>(null);
  const adminUsersBusy = ref(false);
  const adminUsersCreateBusy = ref(false);

  async function loadAdminUsers() {
    if (!authSessionState.user?.is_admin) {
      adminUsers.value = [];
      return;
    }

    adminUsersBusy.value = true;
    try {
      const response = await apiClient.listAdminUsers({
        page: adminUsersPageNumber.value,
        page_size: adminUsersPageSize.value,
        query: adminUsersQuery.value.trim() || undefined,
        sort_by: adminUsersSort.value?.field,
        sort_direction: adminUsersSort.value?.direction,
      });
      adminUsersPage.value = response;
      adminUsers.value = response.items;
      adminUsersPageNumber.value = response.pagination.page;
      adminUsersPageSize.value = response.pagination.page_size;
    } catch (error) {
      showErrorToast(error, t("adminUsers.loadFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  function changeAdminUsersPage(page: number) {
    adminUsersPageNumber.value = page;
    void loadAdminUsers();
  }

  function changeAdminUsersPageSize(value: number) {
    if (adminUsersPageSize.value === value) return;
    adminUsersPageSize.value = value;
    adminUsersPageNumber.value = 1;
    void loadAdminUsers();
  }

  function changeAdminUsersSort(sort: AdminUserSort | null) {
    if (adminUsersSort.value?.field === sort?.field && adminUsersSort.value?.direction === sort?.direction) return;
    adminUsersSort.value = sort;
    adminUsersPageNumber.value = 1;
    void loadAdminUsers();
  }

  async function createAdminUser(payload: {
    login_name: string;
    display_name: string;
    password: string;
    is_admin: boolean;
  }) {
    adminUsersCreateBusy.value = true;
    try {
      await apiClient.createAdminUser(payload);
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.createFailed"));
    } finally {
      adminUsersCreateBusy.value = false;
    }
  }

  async function updateAdminUser(payload: {
    login_name: string;
    display_name: string;
    is_admin: boolean;
  }) {
    adminUsersBusy.value = true;
    try {
      await apiClient.updateAdminUser(payload.login_name, {
        display_name: payload.display_name,
        is_admin: payload.is_admin,
      });
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.updateFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function resetAdminUserPassword(payload: {
    login_name: string;
    password: string;
  }) {
    adminUsersBusy.value = true;
    try {
      await apiClient.resetAdminUserPassword(payload.login_name, {
        password: payload.password,
      });
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.resetFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function disableAdminUser(loginName: string) {
    adminUsersBusy.value = true;
    try {
      await apiClient.disableAdminUser(loginName);
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.disableFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function enableAdminUser(loginName: string) {
    adminUsersBusy.value = true;
    try {
      await apiClient.enableAdminUser(loginName);
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.enableFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  return {
    adminUsers,
    adminUsersBusy,
    adminUsersCreateBusy,
    adminUsersPage,
    adminUsersPageNumber,
    adminUsersPageSize,
    adminUsersQuery,
    changeAdminUsersPage,
    changeAdminUsersPageSize,
    changeAdminUsersSort,
    createAdminUser,
    disableAdminUser,
    enableAdminUser,
    loadAdminUsers,
    resetAdminUserPassword,
    updateAdminUser,
  };
}
