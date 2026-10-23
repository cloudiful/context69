import { useQuery } from "@tanstack/react-query";
import { ChevronDown, ChevronUp, Search, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { toast } from "sonner";

import { TablePagination } from "@/components/common/table-pagination";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatTimestamp } from "@/lib/format";
import { apiClient } from "@/services/api";
import type { GroupKind, GroupResponse, GroupSortBy, Visibility } from "@/services/api";

import { GroupEntityDialog } from "./group-dialogs";
import { visibilityLabel } from "@/lib/visibility-label";

import { groupKindLabel, membershipRoleLabel } from "./group-labels";
import { groupPathOf, roleBadgeVariant } from "./permissions";

const PAGE_SIZE = 50;

export function GroupsListPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [visibility, setVisibility] = useState<Visibility | null>(null);
  const [kind, setKind] = useState<GroupKind | null>(null);
  const [sort, setSort] = useState<{ field: GroupSortBy; direction: "asc" | "desc" } | null>(null);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [createOpen, setCreateOpen] = useState(false);
  const [createBusy, setCreateBusy] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(search.trim());
      setPage(1);
    }, 250);
    return () => clearTimeout(timer);
  }, [search]);

  const groupsQuery = useQuery({
    queryKey: [
      "groups",
      page,
      pageSize,
      debouncedSearch,
      visibility ?? "",
      kind ?? "",
      sort?.field ?? "",
      sort?.direction ?? "",
    ],
    queryFn: ({ signal }) => apiClient.listGroups({
      page,
      page_size: pageSize,
      query: debouncedSearch || undefined,
      visibility: visibility ?? undefined,
      kind: kind ?? undefined,
      sort_by: sort?.field,
      sort_direction: sort?.direction,
    }, { signal }),
    retry: false,
  });

  const groups = groupsQuery.data?.items ?? [];
  const pagination = groupsQuery.data?.pagination ?? null;

  function openGroup(group: GroupResponse) {
    navigate(`/groups/${encodeURIComponent(groupPathOf(group))}`);
  }

  async function createGroup(draft: { key?: string; name: string; visibility: Visibility }) {
    setCreateBusy(true);
    try {
      await apiClient.createGroup({
        group_key: draft.key?.trim() ?? "",
        name: draft.name.trim(),
        visibility: draft.visibility,
        kind: "shared",
      });
      setCreateOpen(false);
      await groupsQuery.refetch();
      toast.success(t("groups.create"), { description: draft.name.trim(), duration: 2500 });
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.createFailed"),
        duration: 5000,
      });
    } finally {
      setCreateBusy(false);
    }
  }

  function toggleSort(field: GroupSortBy) {
    setSort((current) => {
      if (current?.field !== field) {
        return { field, direction: "asc" };
      }
      return { field, direction: current.direction === "asc" ? "desc" : "asc" };
    });
    setPage(1);
  }

  const sortIcon = (field: GroupSortBy) => {
    if (sort?.field !== field) {
      return null;
    }
    return sort.direction === "asc" ? <ChevronUp aria-hidden="true" /> : <ChevronDown aria-hidden="true" />;
  };

  const error = groupsQuery.error;

  return (
    <div className="grid min-w-0 gap-3" data-testid="groups-list">
      <h1 className="text-lg font-semibold">{t("nav.groups")}</h1>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative w-64 max-w-full">
            <Search
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              className="pl-8"
              value={search}
              aria-label={t("groups.groupName")}
              placeholder={t("groups.groupName")}
              data-testid="groups-search"
              onChange={(event) => setSearch(event.target.value)}
            />
          </div>
          <Select
            value={visibility ?? "__all__"}
            onValueChange={(next) => {
              setVisibility(next === "__all__" ? null : next as Visibility);
              setPage(1);
            }}
          >
            <SelectTrigger className="w-40" aria-label={t("groups.visibility")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("groups.allVisibilities")}</SelectItem>
              <SelectItem value="private">{t("groups.visibilityOptions.private")}</SelectItem>
              <SelectItem value="public">{t("groups.visibilityOptions.public")}</SelectItem>
            </SelectContent>
          </Select>
          <Select
            value={kind ?? "__all__"}
            onValueChange={(next) => {
              setKind(next === "__all__" ? null : next as GroupKind);
              setPage(1);
            }}
          >
            <SelectTrigger className="w-40" aria-label={t("groups.kind")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("groups.allKinds")}</SelectItem>
              <SelectItem value="shared">{t("groups.kindOptions.shared")}</SelectItem>
              <SelectItem value="personal">{t("groups.kindOptions.personal")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <Button type="button" data-testid="groups-create" onClick={() => setCreateOpen(true)}>
          {t("groups.create")}
        </Button>
      </div>

      {error ? (
        <div role="alert" className="flex flex-wrap items-center gap-2 rounded-md border border-destructive/40 p-3 text-sm">
          <span>{error instanceof Error ? error.message : t("groups.loadFailed")}</span>
          <Button type="button" size="sm" variant="outline" onClick={() => void groupsQuery.refetch()}>
            {t("common.retry")}
          </Button>
        </div>
      ) : null}

      <div className="min-w-0 overflow-x-auto">
        <Table data-testid="groups-table">
          <TableHeader>
            <TableRow>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={() => toggleSort("group_key")}>
                  {t("groups.groupKey")} {sortIcon("group_key")}
                </button>
              </TableHead>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={() => toggleSort("name")}>
                  {t("groups.groupName")} {sortIcon("name")}
                </button>
              </TableHead>
              <TableHead>{t("groups.visibility")}</TableHead>
              <TableHead>{t("groups.kind")}</TableHead>
              <TableHead>{t("groups.currentRole")}</TableHead>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={() => toggleSort("created_at")}>
                  {t("groups.createdAt")} {sortIcon("created_at")}
                </button>
              </TableHead>
              <TableHead>{t("common.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {groupsQuery.isLoading && groups.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="py-8 text-sm text-muted-foreground">{t("common.loading")}</TableCell>
              </TableRow>
            ) : groups.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="py-8 text-sm text-muted-foreground" data-testid="groups-empty">
                  {t("groups.emptyMessage")}
                </TableCell>
              </TableRow>
            ) : groups.map((group) => (
              <TableRow
                key={group.group_id}
                data-testid="groups-row"
                className="cursor-pointer"
                onClick={() => openGroup(group)}
              >
                <TableCell className="font-mono text-xs">{group.group_key}</TableCell>
                <TableCell>
                  <span className="inline-flex min-w-0 items-center gap-1.5">
                    <Users aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
                    <span className="truncate text-sm font-semibold">{group.name}</span>
                  </span>
                </TableCell>
                <TableCell>
                  <Badge variant="secondary">{visibilityLabel(t, group.visibility)}</Badge>
                </TableCell>
                <TableCell><Badge variant="outline">{groupKindLabel(t, group.kind)}</Badge></TableCell>
                <TableCell>
                  <Badge variant={roleBadgeVariant(group.current_role)}>
                    {membershipRoleLabel(t, group.current_role)}
                  </Badge>
                </TableCell>
                <TableCell className="whitespace-nowrap text-sm text-muted-foreground">
                  {formatTimestamp(group.created_at)}
                </TableCell>
                <TableCell>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={(event) => {
                      event.stopPropagation();
                      openGroup(group);
                    }}
                  >
                    {t("common.open")}
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      {pagination ? (
        <TablePagination
          pagination={pagination}
          onPage={setPage}
          onPageSize={(size) => {
            setPageSize(size);
            setPage(1);
          }}
        />
      ) : null}

      <GroupEntityDialog
        open={createOpen}
        title={t("groups.create")}
        busy={createBusy}
        submitLabel={t("groups.create")}
        showKey
        onOpenChange={setCreateOpen}
        onSubmit={(draft) => void createGroup(draft)}
      />
    </div>
  );
}
