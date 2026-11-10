import { getCoreRowModel, useReactTable, type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { GitRepositoryFile } from "@/services/api";

import { renderTable } from "./table";

export function FilesTable({
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
