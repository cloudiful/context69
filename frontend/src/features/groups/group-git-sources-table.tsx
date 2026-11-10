import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type {
  GitIndexStatus,
  GitProviderConnection,
  GitRepositoryFile,
  GitRepositorySource,
} from "@/services/api";

function indexTone(status: GitIndexStatus): StatusTone {
  switch (status) {
    case "indexing":
    case "pending":
      return "active";
    case "ready":
      return "success";
    case "stale":
      return "warning";
    case "failed":
      return "failure";
    default:
      return "neutral";
  }
}

function shortSha(value: string | null | undefined): string {
  if (!value) {
    return "--";
  }
  return value.slice(0, 10);
}

function skippedTotal(source: GitRepositorySource): number {
  const summary = source.last_skip_summary ?? {};
  return (summary.excluded_directory ?? 0)
    + (summary.binary ?? 0)
    + (summary.minified ?? 0)
    + (summary.source_map ?? 0)
    + (summary.lockfile ?? 0)
    + (summary.secret ?? 0)
    + (summary.too_large ?? 0)
    + (summary.excluded_by_user ?? 0)
    + (summary.not_included ?? 0);
}

function formatDate(value: string | null | undefined): string {
  if (!value) {
    return "--";
  }
  return new Date(value).toLocaleString();
}

function renderTable<T>(table: ReturnType<typeof useReactTable<T>>, emptyLabel: string, rowTestId: string) {
  return (
    <Table>
      <TableHeader>
        {table.getHeaderGroups().map((headerGroup) => (
          <TableRow key={headerGroup.id}>
            {headerGroup.headers.map((header) => (
              <TableHead key={header.id}>
                {header.isPlaceholder
                  ? null
                  : flexRender(header.column.columnDef.header, header.getContext())}
              </TableHead>
            ))}
          </TableRow>
        ))}
      </TableHeader>
      <TableBody>
        {table.getRowModel().rows.length === 0 ? (
          <TableRow>
            <TableCell colSpan={table.getAllColumns().length} className="py-6 text-sm text-muted-foreground">
              {emptyLabel}
            </TableCell>
          </TableRow>
        ) : table.getRowModel().rows.map((row) => (
          <TableRow key={row.id} data-testid={rowTestId} data-selected={row.getIsSelected()}>
            {row.getVisibleCells().map((cell) => (
              <TableCell key={cell.id}>
                {flexRender(cell.column.columnDef.cell, cell.getContext())}
              </TableCell>
            ))}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

export function GitSourcesTable({
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
                {t("groups.gitSources.configuredPin")}: {row.original.version.commit_sha.slice(0, 10)}
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
            <StatusBadge
              tone={indexTone(row.original.index_status)}
              title={t(`groups.gitSources.indexStatus.${row.original.index_status}`)}
            >
              {t(`groups.gitSources.indexStatus.${row.original.index_status}`)}
            </StatusBadge>
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
            <span>{t("groups.gitSources.targetSha")}: {shortSha(row.original.checkpoint.target_commit_sha)}</span>
            <span>{t("groups.gitSources.indexedSha")}: {shortSha(row.original.checkpoint.indexed_commit_sha)}</span>
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
        cell: ({ row }) => <span className="text-sm">{skippedTotal(row.original)}</span>,
      },
      {
        accessorKey: "updated_at",
        header: t("groups.gitSources.updated"),
        cell: ({ row }) => <span className="text-xs">{formatDate(row.original.updated_at)}</span>,
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

export function GitConnectionsTable({
  connections,
  canManage,
  busyKey,
  onToggle,
}: {
  connections: GitProviderConnection[];
  canManage: boolean;
  busyKey: string | null;
  onToggle: (connection: GitProviderConnection) => void;
}) {
  const { t } = useTranslation();

  const columns = useMemo<ColumnDef<GitProviderConnection>[]>(() => [
    {
      accessorKey: "display_name",
      header: t("groups.gitSources.connectionName"),
      cell: ({ row }) => (
        <div className="grid gap-0.5">
          <span className="text-sm font-medium">{row.original.display_name}</span>
          <span className="font-mono text-xs text-muted-foreground">{row.original.connection_key}</span>
        </div>
      ),
    },
    {
      accessorKey: "provider",
      header: t("groups.gitSources.provider"),
      cell: ({ row }) => <span className="text-sm capitalize">{row.original.provider}</span>,
    },
    {
      accessorKey: "mode",
      header: t("groups.gitSources.connectionMode"),
      cell: ({ row }) => <span className="text-sm">{row.original.mode}</span>,
    },
    {
      accessorKey: "base_url",
      header: t("groups.gitSources.connectionBaseUrl"),
      cell: ({ row }) => <span className="font-mono text-xs">{row.original.base_url}</span>,
    },
    {
      id: "credentials",
      header: t("groups.gitSources.connectionCredentials"),
      cell: ({ row }) => (
        <span className="text-xs">
          {row.original.has_read_credential
            ? t("groups.gitSources.readCredentialPresent")
            : t("groups.gitSources.readCredentialAbsent")}
          {" · "}
          {row.original.has_webhook_secret
            ? t("groups.gitSources.webhookSecretPresent")
            : t("groups.gitSources.webhookSecretAbsent")}
        </span>
      ),
    },
    {
      id: "state",
      header: t("groups.gitSources.state"),
      cell: ({ row }) => (
        <StatusBadge tone={row.original.disabled ? "neutral" : "success"}>
          {row.original.disabled ? t("groups.gitSources.connectionDisabled") : t("groups.gitSources.connectionEnabled")}
        </StatusBadge>
      ),
    },
    {
      id: "actions",
      header: t("common.actions"),
      cell: ({ row }) => canManage ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={busyKey === row.original.connection_key}
          onClick={() => onToggle(row.original)}
        >
          {row.original.disabled ? t("groups.gitSources.enable") : t("groups.gitSources.disable")}
        </Button>
      ) : null,
    },
  ], [busyKey, canManage, onToggle, t]);

  const table = useReactTable({
    data: connections,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  return <div className="min-w-0 overflow-x-auto" data-testid="git-connections-table">{renderTable(table, t("groups.gitSources.emptyConnections"), "git-connection-row")}</div>;
}

export function GitFilesTable({
  files,
  selectedPath,
  onSelect,
}: {
  files: GitRepositoryFile[];
  selectedPath: string | null;
  onSelect: (file: GitRepositoryFile) => void;
}) {
  const { t } = useTranslation();

  const columns = useMemo<ColumnDef<GitRepositoryFile>[]>(() => [
    {
      accessorKey: "path",
      header: t("groups.gitSources.filePath"),
      cell: ({ row }) => (
        <button
          type="button"
          className="max-w-96 truncate text-left font-mono text-xs hover:underline"
          onClick={() => onSelect(row.original)}
        >
          {row.original.path}
        </button>
      ),
    },
    {
      accessorKey: "language",
      header: t("groups.gitSources.fileLanguage"),
      cell: ({ row }) => <span className="text-xs">{row.original.language}</span>,
    },
    {
      accessorKey: "line_count",
      header: t("groups.gitSources.fileLines"),
      cell: ({ row }) => <span className="text-xs">{row.original.line_count}</span>,
    },
    {
      accessorKey: "byte_count",
      header: t("groups.gitSources.fileBytes"),
      cell: ({ row }) => <span className="text-xs">{row.original.byte_count}</span>,
    },
  ], [onSelect, t]);

  const table = useReactTable({
    data: files,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  return (
    <div className="min-w-0 overflow-x-auto" data-testid="git-files-table" data-selected-path={selectedPath ?? ""}>
      {renderTable(table, t("groups.gitSources.emptyFiles"), "git-file-row")}
    </div>
  );
}
