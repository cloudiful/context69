import { getCoreRowModel, useReactTable, type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { StatusBadge } from "@/components/common/status-badge";
import { Button } from "@/components/ui/button";
import type { GitRepositorySource } from "@/services/api";
import { formatTimestamp } from "@/lib/format";

import { GitIndexStatusBadge, gitSkipTotal, shortSha } from "../presentation";
import { renderTable } from "./table";

export function SourcesTable({
  sources,
  canManage,
  busyKey,
  onSelect,
  onSync,
  onRebuild,
  onEdit,
  onToggleEnabled,
  onWebhook,
  onDelete,
}: {
  sources: GitRepositorySource[];
  canManage: boolean;
  busyKey: string | null;
  onSelect: (source: GitRepositorySource) => void;
  onSync: (source: GitRepositorySource) => void;
  onRebuild: (source: GitRepositorySource) => void;
  onEdit: (source: GitRepositorySource) => void;
  onToggleEnabled: (source: GitRepositorySource) => void;
  onWebhook: (source: GitRepositorySource) => void;
  onDelete: (source: GitRepositorySource) => void;
}) {
  const { t } = useTranslation();

  const columns = useMemo<ColumnDef<GitRepositorySource>[]>(() => {
    const base: ColumnDef<GitRepositorySource>[] = [
      {
        id: "repository",
        header: t("groups.gitSources.repository"),
        cell: ({ row }) => (
          <button
            type="button"
            className="grid max-w-72 gap-0.5 text-left"
            onClick={() => onSelect(row.original)}
          >
            <span className="truncate text-sm font-medium">{row.original.owner}/{row.original.name}</span>
            <span className="truncate font-mono text-xs text-muted-foreground">
              {row.original.canonical_url}
            </span>
          </button>
        ),
      },
      {
        accessorKey: "provider",
        header: t("groups.gitSources.provider"),
        cell: ({ row }) => <span className="text-sm capitalize">{row.original.provider}</span>,
      },
      {
        id: "ref",
        header: t("groups.gitSources.ref"),
        cell: ({ row }) => (
          <div className="grid gap-0.5">
            <span className="font-mono text-xs">{row.original.version.ref_name}</span>
            {row.original.version.commit_sha ? (
              <span className="font-mono text-xs text-muted-foreground">
                {t("groups.gitSources.configuredPin")}: {shortSha(row.original.version.commit_sha, 10)}
              </span>
            ) : null}
          </div>
        ),
      },
      {
        id: "state",
        header: t("groups.gitSources.state"),
        cell: ({ row }) => (
          <div className="flex flex-wrap items-center gap-1">
            <GitIndexStatusBadge status={row.original.index_status} />
            {row.original.refresh_pending ? (
              <StatusBadge tone="info">{t("groups.gitSources.refreshPending")}</StatusBadge>
            ) : null}
            {row.original.deletion_pending ? (
              <StatusBadge tone="warning">{t("groups.gitSources.deletionPending")}</StatusBadge>
            ) : null}
            {row.original.last_error_code ? (
              <StatusBadge tone="failure" title={row.original.last_error_code}>
                {row.original.last_error_code}
              </StatusBadge>
            ) : null}
          </div>
        ),
      },
      {
        id: "sha",
        header: t("groups.gitSources.sha"),
        cell: ({ row }) => (
          <div className="grid gap-0.5 font-mono text-xs">
            <span>{t("groups.gitSources.targetSha")}: {shortSha(row.original.checkpoint.target_commit_sha, 10)}</span>
            <span>{t("groups.gitSources.indexedSha")}: {shortSha(row.original.checkpoint.indexed_commit_sha, 10)}</span>
          </div>
        ),
      },
      {
        id: "refresh",
        header: t("groups.gitSources.refresh"),
        cell: ({ row }) => (
          <div className="grid gap-0.5 text-xs">
            <span>
              {row.original.interval_seconds
                ? t("groups.gitSources.intervalEvery", { seconds: row.original.interval_seconds })
                : t("groups.gitSources.manualOnly")}
            </span>
            <span className="text-muted-foreground">
              {row.original.webhook_enabled ? t("groups.gitSources.webhookOn") : t("groups.gitSources.webhookOff")}
            </span>
          </div>
        ),
      },
      {
        id: "skipped",
        header: t("groups.gitSources.skipped"),
        cell: ({ row }) => <span className="text-sm">{gitSkipTotal(row.original)}</span>,
      },
      {
        accessorKey: "updated_at",
        header: t("groups.gitSources.updated"),
        cell: ({ row }) => <span className="text-xs">{formatTimestamp(row.original.updated_at)}</span>,
      },
      {
        id: "actions",
        header: t("common.actions"),
        cell: ({ row }) => {
          const source = row.original;
          const busy = busyKey === source.repository_key;
          return (
            <div className="flex flex-wrap gap-1">
              <Button type="button" size="sm" variant="outline" onClick={() => onSelect(source)}>
                {t("groups.gitSources.browse")}
              </Button>
              {canManage ? (
                <>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={busy || !source.enabled}
                    onClick={() => onSync(source)}
                  >
                    {t("groups.gitSources.sync")}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() => onRebuild(source)}
                  >
                    {t("groups.gitSources.rebuild")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" onClick={() => onEdit(source)}>
                    {t("common.edit")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" onClick={() => onWebhook(source)}>
                    {t("groups.gitSources.webhook")}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => onToggleEnabled(source)}
                  >
                    {source.enabled ? t("groups.gitSources.disable") : t("groups.gitSources.enable")}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="text-destructive"
                    disabled={busy}
                    onClick={() => onDelete(source)}
                  >
                    {t("groups.gitSources.delete")}
                  </Button>
                </>
              ) : null}
            </div>
          );
        },
      },
    ];
    return base;
  }, [busyKey, canManage, onDelete, onEdit, onRebuild, onSelect, onSync, onToggleEnabled, onWebhook, t]);

  const table = useReactTable({
    data: sources,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  return <div className="min-w-0 overflow-x-auto" data-testid="git-sources-table">{renderTable(table, t("groups.gitSources.empty"), "git-source-row")}</div>;
}
