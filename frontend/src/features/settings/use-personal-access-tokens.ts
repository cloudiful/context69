import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type {
  CreatePersonalAccessTokenResponse,
  Pagination,
  PersonalAccessTokenResponse,
  PersonalAccessTokenScope,
} from "@/services/api";

import { errorMessage } from "./settings-error";

const DEFAULT_SCOPES: PersonalAccessTokenScope[] = ["search"];
const DEFAULT_EXPIRY_DAYS = 30;
const PAGE_SIZE = 50;

/**
 * The token lifetimes the product offers. The value is the wire contract
 * (`expires_in_days`); its label lives under
 * `settings.personalAccessTokens.expiry<Days>Days` in every locale.
 */
export const PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS = [7, 30, 90, 365] as const;

export interface PersonalAccessTokenDraft {
  name: string;
  scopes: PersonalAccessTokenScope[];
  expires_in_days: number;
}

const EMPTY_PAGINATION: Pagination = { page: 1, page_size: PAGE_SIZE, total: 0, total_pages: 0 };

export const PERSONAL_ACCESS_TOKEN_SCOPES: PersonalAccessTokenScope[] = [
  "search",
  "workspace",
  "library",
  "sources",
  "settings",
  "admin",
];

/**
 * Access-token list, create, one-time reveal, copy and revoke. The revealed
 * secret lives only in this component state; it is never written to storage,
 * logs, or a test artifact.
 */
export function usePersonalAccessTokens() {
  const { t } = useTranslation();

  const [items, setItems] = useState<PersonalAccessTokenResponse[]>([]);
  const [pagination, setPagination] = useState<Pagination>(EMPTY_PAGINATION);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [reveal, setReveal] = useState<CreatePersonalAccessTokenResponse | null>(null);
  const [draft, setDraft] = useState<PersonalAccessTokenDraft>({
    name: "",
    scopes: [...DEFAULT_SCOPES],
    expires_in_days: DEFAULT_EXPIRY_DAYS,
  });
  const [revokeTarget, setRevokeTarget] = useState<PersonalAccessTokenResponse | null>(null);

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

  const validationError = !draft.name.trim()
    ? t("settings.personalAccessTokens.nameRequired")
    : draft.scopes.length === 0
      ? t("settings.personalAccessTokens.scopesRequired")
      : "";

  const updateDraft = useCallback((patch: Partial<PersonalAccessTokenDraft>) => {
    setDraft((current) => ({ ...current, ...patch, scopes: patch.scopes ? [...patch.scopes] : current.scopes }));
  }, []);

  const create = useCallback(async () => {
    if (validationError) {
      showErrorToast(null, validationError);
      return;
    }

    setCreating(true);
    try {
      const created = await apiClient.createPersonalAccessToken({
        name: draft.name.trim(),
        scopes: draft.scopes,
        expires_in_days: draft.expires_in_days,
      });
      setReveal(created);
      await load(1, pageSize);
      setPage(1);
      setDraft({ name: "", scopes: [...DEFAULT_SCOPES], expires_in_days: DEFAULT_EXPIRY_DAYS });
      toast.success(t("settings.personalAccessTokens.createSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.createFailed"));
    } finally {
      setCreating(false);
    }
  }, [draft, load, pageSize, showErrorToast, t, validationError]);

  const revoke = useCallback(async (token: PersonalAccessTokenResponse) => {
    try {
      await apiClient.revokePersonalAccessToken(token.token_id);
      await load(page, pageSize);
      toast.success(t("settings.personalAccessTokens.revokeSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.revokeFailed"));
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
    changePage,
    changePageSize,
    copy,
    creating,
    create,
    dismissReveal,
    draft,
    items,
    loading,
    pagination,
    reveal,
    revoke,
    revokeTarget,
    setRevokeTarget,
    updateDraft,
    validationError,
  };
}

export type PersonalAccessTokensState = ReturnType<typeof usePersonalAccessTokens>;
