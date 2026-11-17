import { Loader2, Search, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { errorMessage } from "@/features/settings/settings-error";
import { useAuthSession } from "@/hooks/use-auth";
import { apiClient, type GroupResponse, type Pagination } from "@/services/api";

import { rememberGroups, useGroupLabelCache } from "./personal-access-token-group-labels";

const GROUP_PAGE_SIZE = 100;
const SEARCH_DEBOUNCE_MS = 300;

function groupPathOf(group: GroupResponse): string {
  return group.group_path ?? group.group_key;
}

/**
 * Flat paged browser over the token-management group catalog. The catalog
 * already contains every accessible group (including a child whose ancestor is
 * inaccessible), the optional filter runs before pagination, and selection is
 * tracked by stable group id across pages and filters.
 */
export function PersonalAccessTokenGroupPicker({
  selectedIds,
  onSelectedIdsChange,
}: {
  selectedIds: number[];
  onSelectedIdsChange: (ids: number[]) => void;
}) {
  const { t } = useTranslation();
  const { user } = useAuthSession();
  const userKey = user ? String(user.user_id) : "anonymous";
  const labels = useGroupLabelCache(userKey);
  const [items, setItems] = useState<GroupResponse[]>([]);
  const [pagination, setPagination] = useState<Pagination | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [query, setQuery] = useState("");
  const [appliedQuery, setAppliedQuery] = useState("");
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    const handle = setTimeout(() => {
      setPage(1);
      setAppliedQuery(query.trim());
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [query]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    void apiClient.listPersonalAccessTokenGroupCatalog({
      page,
      pageSize: GROUP_PAGE_SIZE,
      query: appliedQuery,
    })
      .then((response) => {
        if (cancelled) {
          return;
        }
        rememberGroups(userKey, response.items);
        setItems(response.items);
        setPagination(response.pagination);
        setPage(response.pagination.page);
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        setItems([]);
        setPagination(null);
        setLoadError(errorMessage(error, t("settings.personalAccessTokens.groupLoadFailed")));
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [appliedQuery, page, reloadKey, t, userKey]);

  function toggleSelected(groupId: number, checked: boolean) {
    if (checked) {
      if (!selectedIds.includes(groupId)) {
        onSelectedIdsChange([...selectedIds, groupId]);
      }
      return;
    }
    onSelectedIdsChange(selectedIds.filter((id) => id !== groupId));
  }

  const totalPages = Math.max(1, pagination?.total_pages ?? 1);

  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap items-center justify-between gap-1.5">
        <span className="text-sm font-medium">{t("settings.personalAccessTokens.groupSelected")}</span>
        <Badge variant="secondary">
          {t("settings.personalAccessTokens.groupSelectedCount", { count: selectedIds.length })}
        </Badge>
      </div>

      {selectedIds.length > 0 ? (
        <div className="flex max-h-24 flex-wrap gap-1 overflow-y-auto">
          {selectedIds.map((id) => {
            const label = labels.get(id);
            const name = label?.name ?? t("settings.personalAccessTokens.groupFallback", { id });
            return (
              <Badge
                key={id}
                variant={label ? "secondary" : "outline"}
                className="max-w-full gap-1 py-0 pr-1"
                title={label?.path}
              >
                <span className="truncate">{name}</span>
                <button
                  type="button"
                  className="rounded p-0.5 hover:bg-background/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  aria-label={t("settings.personalAccessTokens.groupRemove", { name })}
                  onClick={() => toggleSelected(id, false)}
                >
                  <X aria-hidden="true" className="size-3" />
                </button>
              </Badge>
            );
          })}
        </div>
      ) : null}

      <div className="relative">
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          value={query}
          className="pl-7"
          placeholder={t("settings.personalAccessTokens.groupSearchPlaceholder")}
          aria-label={t("settings.personalAccessTokens.groupSearchPlaceholder")}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>

      {appliedQuery.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          {t("settings.personalAccessTokens.groupFilterHint", { query: appliedQuery })}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          {t("settings.personalAccessTokens.groupCatalogHint")}
        </p>
      )}

      <div className="max-h-64 min-h-24 overflow-y-auto rounded-md border">
        {loading ? (
          <div role="status" className="flex items-center gap-2 p-3 text-sm text-muted-foreground">
            <Loader2 aria-hidden="true" className="size-4 animate-spin" />
            {t("settings.personalAccessTokens.groupLoading")}
          </div>
        ) : loadError ? (
          <div className="grid gap-2 p-3 text-sm">
            <span className="text-destructive">{loadError}</span>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="w-fit"
              onClick={() => setReloadKey((current) => current + 1)}
            >
              {t("settings.personalAccessTokens.groupRetry")}
            </Button>
          </div>
        ) : items.length === 0 ? (
          <p className="p-3 text-sm text-muted-foreground">{t("settings.personalAccessTokens.groupEmpty")}</p>
        ) : (
          items.map((group) => {
            const checkboxId = `pat-group-${group.group_id}`;
            return (
              <div key={group.group_id} className="flex items-start gap-2 border-b px-2 py-1.5 last:border-b-0">
                <Checkbox
                  id={checkboxId}
                  className="mt-0.5"
                  checked={selectedIds.includes(group.group_id)}
                  onCheckedChange={(checked) => toggleSelected(group.group_id, checked === true)}
                />
                <label htmlFor={checkboxId} className="grid min-w-0 cursor-pointer gap-0.5">
                  <span className="truncate text-sm">{group.name}</span>
                  <span className="truncate text-xs text-muted-foreground">{groupPathOf(group)}</span>
                </label>
              </div>
            );
          })
        )}
      </div>

      {!loading && !loadError && totalPages > 1 ? (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">
            {t("settings.personalAccessTokens.groupPage", { page, total: totalPages })}
          </span>
          <div className="flex gap-1">
            <Button
              type="button"
              size="xs"
              variant="outline"
              disabled={page <= 1}
              onClick={() => setPage((current) => Math.max(1, current - 1))}
            >
              {t("settings.personalAccessTokens.groupPrevious")}
            </Button>
            <Button
              type="button"
              size="xs"
              variant="outline"
              disabled={page >= totalPages}
              onClick={() => setPage((current) => Math.min(totalPages, current + 1))}
            >
              {t("settings.personalAccessTokens.groupNext")}
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
