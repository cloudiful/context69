import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type {
  CreatePersonalAccessTokenRequest,
  CreatePersonalAccessTokenResponse,
  Pagination,
  PersonalAccessTokenResponse,
  PersonalAccessTokenScope,
  UpdatePersonalAccessTokenRequest,
} from "@/services/api";

import { errorMessage } from "./settings-error";

const DEFAULT_EXPIRY_DAYS = 30;
const PAGE_SIZE = 50;

/**
 * The token lifetimes the product offers. The value is the wire contract
 * (`expires_in_days`); its label lives under
 * `settings.personalAccessTokens.expiry<Days>Days` in every locale.
 */
export const PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS = [7, 30, 90, 365] as const;

const EMPTY_PAGINATION: Pagination = { page: 1, page_size: PAGE_SIZE, total: 0, total_pages: 0 };

export const PERSONAL_ACCESS_TOKEN_SCOPES: PersonalAccessTokenScope[] = [
  "search",
  "workspace",
  "library",
  "sources",
  "settings",
  "admin",
];

export type PersonalAccessTokenBusyAction = "create" | "update" | "rotate" | "revoke" | null;

/**
 * Access-token list plus create, edit, rotate, revoke and one-time reveal.
 * The revealed secret lives only in this state; it is never written to
 * storage, logs, or a test artifact.
 */
export function usePersonalAccessTokens() {
  const { t } = useTranslation();

  const [items, setItems] = useState<PersonalAccessTokenResponse[]>([]);
  const [pagination, setPagination] = useState<Pagination>(EMPTY_PAGINATION);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [busyAction, setBusyAction] = useState<PersonalAccessTokenBusyAction>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [reveal, setReveal] = useState<CreatePersonalAccessTokenResponse | null>(null);

  const showErrorToast = useCallback((error: unknown, fallback: string) => {
    toast.error(t("common.error"), {
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  }, [t]);

  const load = useCallback(async (requestPage = page, requestPageSize = pageSize) => {
    setLoading(true);
    try {
      const response = await apiClient.listPersonalAccessTokens({
        page: requestPage,
        pageSize: requestPageSize,
      });
      setItems(response.items);
      setPagination(response.pagination);
      setPage(response.pagination.page);
      setPageSize(response.pagination.page_size);
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.loadFailed"));
    } finally {
      setLoading(false);
    }
  }, [page, pageSize, showErrorToast, t]);

  useEffect(() => {
    void load(1, PAGE_SIZE);
  }, []);

  const changePage = useCallback((next: number) => {
    setPage(next);
    void load(next, pageSize);
  }, [load, pageSize]);

  const changePageSize = useCallback((next: number) => {
    if (next === pageSize) {
      return;
    }
    setPageSize(next);
    setPage(1);
    void load(1, next);
  }, [load, pageSize]);

  const clearActionError = useCallback(() => setActionError(null), []);

  const runAction = useCallback(async (
    action: Exclude<PersonalAccessTokenBusyAction, null>,
    failureKey: string,
    task: () => Promise<void>,
  ): Promise<boolean> => {
    setBusyAction(action);
    setActionError(null);
    try {
      await task();
      return true;
    } catch (error) {
      const message = errorMessage(error, t(failureKey));
      setActionError(message);
      showErrorToast(error, t(failureKey));
      return false;
    } finally {
      setBusyAction(null);
    }
  }, [showErrorToast, t]);

  const create = useCallback(async (payload: CreatePersonalAccessTokenRequest): Promise<boolean> => {
    const ok = await runAction("create", "settings.personalAccessTokens.createFailed", async () => {
      const created = await apiClient.createPersonalAccessToken(payload);
      setReveal(created);
      await load(1, PAGE_SIZE);
    });
    if (ok) {
      toast.success(t("settings.personalAccessTokens.createSuccess"), { duration: 2500 });
    }
    return ok;
  }, [load, runAction, t]);

  const update = useCallback(async (
    tokenId: string,
    payload: UpdatePersonalAccessTokenRequest,
  ): Promise<boolean> => {
    const ok = await runAction("update", "settings.personalAccessTokens.updateFailed", async () => {
      await apiClient.updatePersonalAccessToken(tokenId, payload);
      await load(page, pageSize);
    });
    if (ok) {
      toast.success(t("settings.personalAccessTokens.updateSuccess"), { duration: 2500 });
    }
    return ok;
  }, [load, page, pageSize, runAction, t]);

  const rotate = useCallback(async (tokenId: string, expiresInDays: number): Promise<boolean> => {
    const ok = await runAction("rotate", "settings.personalAccessTokens.rotateFailed", async () => {
      const created = await apiClient.rotatePersonalAccessToken(tokenId, {
        expires_in_days: expiresInDays,
      });
      setReveal(created);
      await load(page, pageSize);
    });
    if (ok) {
      toast.success(t("settings.personalAccessTokens.rotateSuccess"), { duration: 2500 });
    }
    return ok;
  }, [load, page, pageSize, runAction, t]);

  const revoke = useCallback(async (token: PersonalAccessTokenResponse) => {
    setBusyAction("revoke");
    setActionError(null);
    try {
      await apiClient.revokePersonalAccessToken(token.token_id);
      await load(page, pageSize);
      toast.success(t("settings.personalAccessTokens.revokeSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.revokeFailed"));
    } finally {
      setBusyAction(null);
    }
  }, [load, page, pageSize, showErrorToast, t]);

  const copy = useCallback(async () => {
    const token = reveal?.access_token;
    if (!token) {
      return;
    }

    try {
      if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
        throw new Error(t("settings.personalAccessTokens.copyUnavailable"));
      }
      await navigator.clipboard.writeText(token);
      toast.success(t("settings.personalAccessTokens.copySuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.copyFailed"));
    }
  }, [reveal, showErrorToast, t]);

  const dismissReveal = useCallback(() => setReveal(null), []);

  return {
    actionError,
    busyAction,
    changePage,
    changePageSize,
    clearActionError,
    copy,
    create,
    dismissReveal,
    items,
    loading,
    pagination,
    reveal,
    revoke,
    rotate,
    update,
  };
}

export type PersonalAccessTokensState = ReturnType<typeof usePersonalAccessTokens>;
export { DEFAULT_EXPIRY_DAYS };
