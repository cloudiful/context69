import { getCoreRowModel, useReactTable, type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { StatusBadge } from "@/components/common/status-badge";
import { Button } from "@/components/ui/button";
import type { GitProviderConnection } from "@/services/api";

import { renderTable } from "./table";

export function ConnectionsTable({
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
