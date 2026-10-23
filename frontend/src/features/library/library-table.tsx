import {
  ChevronDown,
  ChevronRight,
  FileText,
  MoreHorizontal,
  RefreshCw,
  Unlink,
  Users,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { TablePagination } from "@/components/common/table-pagination";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatBytes, formatTimestamp } from "@/lib/format";
import { cn } from "@/lib/utils";
import { visibilityLabel } from "@/lib/visibility-label";
import type { GroupPageResponse, GroupResponse, LibraryResourceSortBy, Pagination } from "@/services/api";

import {
  canMutateEntry,
  libraryStatusLabel,
  type ExplorerEntry,
  type GroupExplorerEntry,
  type LibraryBrowserEntry,
} from "./library-model";

const TYPE_KEYS: Record<LibraryBrowserEntry["kind"], string> = {
  group: "groups.groupType",
  folder: "library.folderType",
  file: "library.fileType",
};

function statusVariant(status: string): "default" | "secondary" | "destructive" {
  if (status === "succeeded") {
    return "default";
  }
  if (status === "failed") {
    return "destructive";
  }
  return "secondary";
}

export interface LibraryTableProps {
  groupPath: string;
  groupEntries: GroupExplorerEntry[];
  childPage: GroupPageResponse | null;
  entries: ExplorerEntry[];
  loading: boolean;
  error: unknown;
  pagination: Pagination | null;
  selectedFolderId: string | null;
  expandedKeys: Record<string, boolean>;
  selectedKeys: Set<string>;
  unavailableFileIds: string[];
  retryingFileIds: string[];
  releasingFileIds: string[];
  sortBy: LibraryResourceSortBy;
  sortDirection: "asc" | "desc";
  resourceQuery: string;
  statusFilter: string | null;
  onToggleSelect: (key: string) => void;
  onToggleFolder: (id: string | null) => void;
  onOpenEntry: (entry: ExplorerEntry) => void;
  onPreviewEntry: (entry: ExplorerEntry) => void;
  onEditSource: (entry: ExplorerEntry) => void;
  onMoveEntry: (entry: ExplorerEntry) => void;
  onDeleteEntry: (entry: ExplorerEntry) => void;
  onSyncSource: (folderId: string | null) => void;
  onReleaseSource: (fileId: string, filename: string) => void;
  onRetryFile: (fileId: string) => void;
  onRefresh: () => void;
  onSort: (field: LibraryResourceSortBy, direction: "asc" | "desc") => void;
  onPage: (page: number) => void;
  onPageSize: (size: number) => void;
  onOpenGroup: (group: GroupResponse) => void;
  onEditGroup: (group: GroupResponse) => void;
  onMoveGroup: (group: GroupResponse) => void;
  onDeleteGroup: (group: GroupResponse) => void;
  onChildPage: (page: number) => void;
  onChildPageSize: (size: number) => void;
  canRetryFile: (entry: ExplorerEntry) => boolean;
  canRelease: (entry: ExplorerEntry) => boolean;
}

interface ContextMenuState {
  x: number;
  y: number;
  entry: ExplorerEntry;
}

export function LibraryTable(props: LibraryTableProps) {
  const { t } = useTranslation();
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

  useEffect(() => {
    if (!contextMenu) {
      return;
    }
    const close = () => setContextMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        close();
      }
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [contextMenu]);

  const sortable = (field: LibraryResourceSortBy) => ({
    active: props.sortBy === field,
    direction: props.sortBy === field ? props.sortDirection : null,
    onClick: () => {
      const nextDirection = props.sortBy === field && props.sortDirection === "asc" ? "desc" : "asc";
      props.onSort(field, nextDirection);
    },
  });

  const selectableEntries = useMemo(
    () => props.entries.filter((entry) => canMutateEntry(entry)),
    [props.entries],
  );
  const allSelected = selectableEntries.length > 0
    && selectableEntries.every((entry) => props.selectedKeys.has(entry.key));

  const rows: LibraryBrowserEntry[] = [...props.groupEntries, ...props.entries];
  const pagination = props.pagination;

  function openEntry(entry: ExplorerEntry) {
    if (entry.kind === "folder") {
      props.onOpenEntry(entry);
      return;
    }
    if (entry.isSourceConfigFile) {
      props.onEditSource(entry);
      return;
    }
    props.onPreviewEntry(entry);
  }

  function openMenu(event: React.MouseEvent, entry: ExplorerEntry) {
    event.preventDefault();
    event.stopPropagation();
    setContextMenu({ x: event.clientX, y: event.clientY, entry });
  }

  const errorMessage = props.error instanceof Error
    ? props.error.message
    : props.error ? t("library.loadFailed") : null;

  return (
    <div className="grid min-w-0 gap-2">
      {contextMenu ? (
        <div
          role="menu"
          aria-label={t("common.actions")}
          className="fixed z-50 grid w-56 gap-0.5 rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          <button
            type="button"
            role="menuitem"
            className="rounded px-2 py-1.5 text-left text-sm hover:bg-accent"
            onClick={() => {
              openEntry(contextMenu.entry);
              setContextMenu(null);
            }}
          >
            {contextMenu.entry.kind === "file" && contextMenu.entry.isSourceConfigFile
              ? t("library.editSourceConfig")
              : contextMenu.entry.kind === "folder" ? t("library.openFolder") : t("library.preview")}
          </button>
          <button
            type="button"
            role="menuitem"
            className="rounded px-2 py-1.5 text-left text-sm hover:bg-accent"
            onClick={() => {
              props.onRefresh();
              setContextMenu(null);
            }}
          >
            {t("sources.refresh")}
          </button>
          {contextMenu.entry.kind === "folder" && contextMenu.entry.isSourceFolder ? (
            <button
              type="button"
              role="menuitem"
              className="rounded px-2 py-1.5 text-left text-sm hover:bg-accent"
              onClick={() => {
                props.onSyncSource(contextMenu.entry.id);
                setContextMenu(null);
              }}
            >
              {t("sources.sync")}
            </button>
          ) : null}
          {contextMenu.entry.kind === "file" && props.canRetryFile(contextMenu.entry) ? (
            <button
              type="button"
              role="menuitem"
              className="rounded px-2 py-1.5 text-left text-sm hover:bg-accent"
              onClick={() => {
                props.onRetryFile(contextMenu.entry.id as string);
                setContextMenu(null);
              }}
            >
              {t("common.retry")}
            </button>
          ) : null}
          {contextMenu.entry.kind === "file" && props.canRelease(contextMenu.entry) ? (
            <button
              type="button"
              role="menuitem"
              className="rounded px-2 py-1.5 text-left text-sm hover:bg-accent"
              onClick={() => {
                props.onReleaseSource(contextMenu.entry.id as string, contextMenu.entry.name);
                setContextMenu(null);
              }}
            >
              {t("library.releaseSource")}
            </button>
          ) : null}
          {canMutateEntry(contextMenu.entry) ? (
            <>
              <button
                type="button"
                role="menuitem"
                className="rounded px-2 py-1.5 text-left text-sm hover:bg-accent"
                onClick={() => {
                  props.onMoveEntry(contextMenu.entry);
                  setContextMenu(null);
                }}
              >
                {t("common.move")}
              </button>
              <button
                type="button"
                role="menuitem"
                className="rounded px-2 py-1.5 text-left text-sm text-destructive hover:bg-accent"
                onClick={() => {
                  props.onDeleteEntry(contextMenu.entry);
                  setContextMenu(null);
                }}
              >
                {t("common.delete")}
              </button>
            </>
          ) : null}
        </div>
      ) : null}

      {errorMessage ? (
        <div role="alert" className="flex flex-wrap items-center gap-2 rounded-md border border-destructive/40 p-3 text-sm">
          <span>{errorMessage}</span>
          <Button type="button" size="sm" variant="outline" onClick={props.onRefresh}>
            {t("common.retry")}
          </Button>
        </div>
      ) : null}

      <div className="min-w-0 overflow-x-auto">
        <Table data-testid="library-table">
          <TableHeader>
            <TableRow>
              <TableHead className="w-8">
                <Checkbox
                  aria-label={t("library.selectAll")}
                  checked={allSelected}
                  disabled={selectableEntries.length === 0}
                  onCheckedChange={() => {
                    for (const entry of selectableEntries) {
                      if (allSelected === props.selectedKeys.has(entry.key)) {
                        props.onToggleSelect(entry.key);
                      }
                    }
                  }}
                />
              </TableHead>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={sortable("name").onClick}>
                  {t("library.filename")}
                </button>
              </TableHead>
              <TableHead>{t("library.typeLabel")}</TableHead>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={sortable("status").onClick}>
                  {t("library.statusLabel")}
                </button>
              </TableHead>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={sortable("size").onClick}>
                  {t("library.sizeLabel")}
                </button>
              </TableHead>
              <TableHead>
                <button type="button" className="inline-flex items-center gap-1" onClick={sortable("updated_at").onClick}>
                  {t("library.updatedColumn")}
                </button>
              </TableHead>
              <TableHead>{t("common.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {props.loading && rows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="py-8 text-sm text-muted-foreground">
                  {t("common.loading")}
                </TableCell>
              </TableRow>
            ) : rows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="py-8 text-sm text-muted-foreground" data-testid="library-empty">
                  {props.resourceQuery || props.statusFilter
                    ? t("library.noMatchingResources")
                    : t("library.emptyFolderMessage")}
                </TableCell>
              </TableRow>
            ) : (
              rows.map((entry) => {
                if (entry.kind === "group") {
                  return (
                    <TableRow key={entry.key} data-testid="library-group-row">
                      <TableCell />
                      <TableCell>
                        <button
                          type="button"
                          className="inline-flex min-w-0 items-center gap-1.5 text-left"
                          onClick={() => props.onOpenGroup(entry.group)}
                        >
                          <Users aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
                          <span className="truncate text-sm font-semibold">{entry.name}</span>
                        </button>
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">{t("groups.groupType")}</TableCell>
                      <TableCell>
                        <Badge variant="secondary">{visibilityLabel(t, entry.visibility)}</Badge>
                      </TableCell>
                      <TableCell className="text-muted-foreground">-</TableCell>
                      <TableCell className="whitespace-nowrap text-sm text-muted-foreground">
                        {entry.updatedAt ? formatTimestamp(entry.updatedAt) : "-"}
                      </TableCell>
                      <TableCell>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button type="button" variant="ghost" size="icon-sm" aria-label={t("common.actions")}>
                              <MoreHorizontal aria-hidden="true" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onSelect={() => props.onOpenGroup(entry.group)}>{t("common.open")}</DropdownMenuItem>
                            <DropdownMenuItem onSelect={() => props.onEditGroup(entry.group)}>{t("common.edit")}</DropdownMenuItem>
                            <DropdownMenuItem onSelect={() => props.onMoveGroup(entry.group)}>{t("common.move")}</DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem variant="destructive" onSelect={() => props.onDeleteGroup(entry.group)}>
                              {t("common.delete")}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  );
                }

                const isSelected = props.selectedKeys.has(entry.key);
                const isCurrentFolder = entry.kind === "folder" && entry.id === props.selectedFolderId;
                return (
                  <TableRow
                    key={entry.key}
                    data-testid="library-entry-row"
                    data-kind={entry.kind}
                    data-selected={isSelected ? "" : undefined}
                    data-folder-current={isCurrentFolder ? "" : undefined}
                    className={cn(isSelected && "bg-muted/50")}
                    onContextMenu={(event) => openMenu(event, entry)}
                  >
                    <TableCell>
                      {canMutateEntry(entry) ? (
                        <Checkbox
                          aria-label={entry.name}
                          checked={isSelected}
                          onCheckedChange={() => props.onToggleSelect(entry.key)}
                        />
                      ) : null}
                    </TableCell>
                    <TableCell>
                      <div className="flex min-w-0 items-start gap-1.5" style={{ paddingLeft: `${entry.depth * 12}px` }}>
                        {entry.kind === "folder" ? (
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon-sm"
                            aria-label={entry.name}
                            aria-expanded={entry.kind === "folder" ? props.expandedKeys[entry.id ?? "__root__"] === true : undefined}
                            onClick={() => props.onToggleFolder(entry.id)}
                          >
                            {entry.kind === "folder" && props.expandedKeys[entry.id ?? "__root__"] ? (
                              <ChevronDown aria-hidden="true" />
                            ) : (
                              <ChevronRight aria-hidden="true" />
                            )}
                            <span className="sr-only">{entry.name}</span>
                          </Button>
                        ) : (
                          <FileText aria-hidden="true" className="mt-1.5 size-4 shrink-0 text-muted-foreground" />
                        )}
                        <button
                          type="button"
                          className="min-w-0 flex-1 text-left"
                          onClick={() => openEntry(entry)}
                          onDoubleClick={() => props.onOpenEntry(entry)}
                        >
                          <span className="block truncate text-sm font-semibold">{entry.name}</span>
                          {entry.kind === "folder" ? (
                            <span className="block text-xs text-muted-foreground">
                              {t("library.treeCounts", { folders: entry.childFolderCount, files: entry.fileCount })}
                            </span>
                          ) : null}
                        </button>
                      </div>
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">{t(TYPE_KEYS[entry.kind])}</TableCell>
                    <TableCell>
                      {entry.kind === "folder" ? (
                        <span className="text-sm text-muted-foreground">
                          {entry.processingCount > 0 ? t("library.folderProcessing", { count: entry.processingCount }) : "-"}
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1.5">
                          <Badge
                            variant={statusVariant(entry.ingestStatus)}
                            title={entry.errorMessage ?? undefined}
                          >
                            {libraryStatusLabel(t, entry.ingestStatus)}
                          </Badge>
                          {props.unavailableFileIds.includes(entry.id) ? (
                            <Unlink aria-hidden="true" className="size-3.5 text-muted-foreground" />
                          ) : null}
                          {props.canRetryFile(entry) ? (
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon-sm"
                              aria-label={t("common.retry")}
                              aria-busy={props.retryingFileIds.includes(entry.id)}
                              onClick={() => props.onRetryFile(entry.id)}
                            >
                              <RefreshCw aria-hidden="true" className={cn(props.retryingFileIds.includes(entry.id) && "animate-spin")} />
                            </Button>
                          ) : null}
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="tabular-nums text-sm text-muted-foreground">
                      {entry.kind === "file" ? formatBytes(entry.sizeBytes) : "-"}
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-sm text-muted-foreground">
                      {entry.updatedAt ? formatTimestamp(entry.updatedAt) : "-"}
                    </TableCell>
                    <TableCell>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button type="button" variant="ghost" size="icon-sm" aria-label={t("common.actions")}>
                            <MoreHorizontal aria-hidden="true" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onSelect={() => openEntry(entry)}>
                            {entry.kind === "folder"
                              ? t("common.open")
                              : entry.isSourceConfigFile ? t("library.editSourceConfig") : t("library.preview")}
                          </DropdownMenuItem>
                          {entry.kind === "folder" && entry.isSourceFolder ? (
                            <DropdownMenuItem onSelect={() => props.onSyncSource(entry.id)}>
                              {t("sources.sync")}
                            </DropdownMenuItem>
                          ) : null}
                          {entry.kind === "file" && props.canRelease(entry) ? (
                            <DropdownMenuItem onSelect={() => props.onReleaseSource(entry.id, entry.name)}>
                              {t("library.releaseSource")}
                            </DropdownMenuItem>
                          ) : null}
                          {canMutateEntry(entry) ? (
                            <>
                              <DropdownMenuSeparator />
                              <DropdownMenuItem onSelect={() => props.onMoveEntry(entry)}>{t("common.move")}</DropdownMenuItem>
                              <DropdownMenuItem variant="destructive" onSelect={() => props.onDeleteEntry(entry)}>
                                {t("common.delete")}
                              </DropdownMenuItem>
                            </>
                          ) : null}
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>

      {pagination ? (
        <TablePagination
          pagination={pagination}
          onPage={props.onPage}
          onPageSize={props.onPageSize}
        />
      ) : null}
      {props.childPage && props.childPage.pagination.total_pages > 1 ? (
        <TablePagination
          pagination={props.childPage.pagination}
          onPage={props.onChildPage}
          onPageSize={props.onChildPageSize}
        />
      ) : null}
    </div>
  );
}
