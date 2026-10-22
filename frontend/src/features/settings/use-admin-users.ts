import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type { AdminUserResponse, AdminUserSortBy, Pagination } from "@/services/api";

import { errorMessage } from "./settings-error";

const PAGE_SIZE = 50;
const EMPTY_PAGINATION: Pagination = { page: 1, page_size: PAGE_SIZE, total: 0, total_pages: 0 };

export type AdminUserSort = { field: AdminUserSortBy; direction: "asc" | "desc" };

/**
 * Admin account list and commands. Every command is an explicit action and
 * reloads the list from the server; nothing here touches the global Save.
 */
export function useAdminUsers(enabled: boolean) {
  const { t } = useTranslation();

  const [users, setUsers] = useState<AdminUserResponse[]>([]);
  const [pagination, setPagination] = useState<Pagination>(EMPTY_PAGINATION);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [query, setQueryState] = useState("");
  const [sort, setSort] = useState<AdminUserSort | null>(null);
  const [busy, setBusy] = useState(false);
  const [createBusy, setCreateBusy] = useState(false);
  const queryRef = useRef(query);
  const sortRef = useRef(sort);
  const pageRef = useRef(page);
  const pageSizeRef = useRef(pageSize);
  queryRef.current = query;
  sortRef.current = sort;
  pageRef.current = page;
  pageSizeRef.current = pageSize;

  const showErrorToast = useCallback((error: unknown, fallback: string) => {
    toast.error(t("common.error"), {
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  }, [t]);

  const load = useCallback(async (
    requestPage = pageRef.current,
    requestPageSize = pageSizeRef.current,
    requestQuery = queryRef.current,
    requestSort = sortRef.current,
  ) => {
    if (!enabled) {
      setUsers([]);
      return;
    }

    setBusy(true);
    try {
      const response = await apiClient.listAdminUsers({
        page: requestPage,
        page_size: requestPageSize,
        query: requestQuery.trim() || undefined,
        sort_by: requestSort?.field,
        sort_direction: requestSort?.direction,
      });
      setPagination(response.pagination);
      setUsers(response.items);
      setPage(response.pagination.page);
      setPageSize(response.pagination.page_size);
    } catch (error) {
      showErrorToast(error, t("adminUsers.loadFailed"));
    } finally {
      setBusy(false);
    }
  }, [enabled, showErrorToast, t]);

  useEffect(() => {
    void load(1, PAGE_SIZE, "", null);
  }, []);

  const setQuery = useCallback((value: string) => {
    setQueryState(value);
  }, []);

  // Debounced search: the field stays responsive while requests follow typing.
  useEffect(() => {
    if (query === "") {
      return;
    }
    const timer = setTimeout(() => {
      void load(1, pageSizeRef.current, query, sortRef.current);
    }, 250);
    return () => clearTimeout(timer);
  }, [load, query]);

  const changePage = useCallback((next: number) => {
    setPage(next);
    void load(next, pageSizeRef.current, queryRef.current, sortRef.current);
  }, [load]);

  const changePageSize = useCallback((next: number) => {
    if (next === pageSizeRef.current) {
      return;
    }
    setPageSize(next);
    setPage(1);
    void load(1, next, queryRef.current, sortRef.current);
  }, [load]);

  const changeSort = useCallback((next: AdminUserSort | null) => {
    if (next?.field === sortRef.current?.field && next?.direction === sortRef.current?.direction) {
      return;
    }
    setSort(next);
    setPage(1);
    void load(1, pageSizeRef.current, queryRef.current, next);
  }, [load]);

  const runCommand = useCallback(async (action: () => Promise<unknown>, fallback: string, create = false) => {
    if (create) {
      setCreateBusy(true);
    } else {
      setBusy(true);
    }
    try {
      await action();
      await load(pageRef.current, pageSizeRef.current, queryRef.current, sortRef.current);
    } catch (error) {
      showErrorToast(error, fallback);
    } finally {
      setCreateBusy(false);
      setBusy(false);
    }
  }, [load, showErrorToast]);

  const createUser = useCallback((payload: {
    login_name: string;
    display_name: string;
    password: string;
    is_admin: boolean;
  }) => runCommand(
    () => apiClient.createAdminUser(payload),
    t("adminUsers.createFailed"),
    true,
  ), [runCommand, t]);

  const updateUser = useCallback((payload: {
    login_name: string;
    display_name: string;
    is_admin: boolean;
  }) => runCommand(
    () => apiClient.updateAdminUser(payload.login_name, {
      display_name: payload.display_name,
      is_admin: payload.is_admin,
    }),
    t("adminUsers.updateFailed"),
  ), [runCommand, t]);

  const resetPassword = useCallback((payload: { login_name: string; password: string }) => runCommand(
    () => apiClient.resetAdminUserPassword(payload.login_name, { password: payload.password }),
    t("adminUsers.resetFailed"),
  ), [runCommand, t]);

  const disableUser = useCallback((loginName: string) => runCommand(
    () => apiClient.disableAdminUser(loginName),
    t("adminUsers.disableFailed"),
  ), [runCommand, t]);

  const enableUser = useCallback((loginName: string) => runCommand(
    () => apiClient.enableAdminUser(loginName),
    t("adminUsers.enableFailed"),
  ), [runCommand, t]);

  return {
    busy,
    changePage,
    changePageSize,
    changeSort,
    createBusy,
    createUser,
    disableUser,
    enableUser,
    pagination,
    query,
    resetPassword,
    setQuery,
    sort,
    updateUser,
    users,
  };
}

export type AdminUsersState = ReturnType<typeof useAdminUsers>;
