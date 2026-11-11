import { ArrowUp, ChevronDown, Plus, RotateCcw, ScanText, Search, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { LibraryIngestStatus } from "@/services/api";

import { libraryStatusLabel, type LibrarySearchMode } from "./library-model";

export function LibraryToolbar({
  trail,
  statusFilter,
  searchMode,
  resourceQuery,
  scopedQuery,
  scopedPlaceholder,
  scopedLoading,
  uploadBusy,
  uploadReady,
  canManage,
  selectedCount,
  retryableSelectedCount,
  retrySelectedBusy,
  onUp,
  onBreadcrumb,
  onStatusFilter,
  onSearchMode,
  onResourceQuery,
  onScopedQuery,
  onScopedSearch,
  onUpload,
  onCreateFolder,
  onCreateText,
  onCreateSource,
  onCreateChildGroup,
  onCreateGitRepository,
  onRetrySelected,
  onDeleteSelected,
}: {
  trail: Array<{ id: string | null; name: string }>;
  statusFilter: LibraryIngestStatus | null;
  searchMode: LibrarySearchMode;
  resourceQuery: string;
  scopedQuery: string;
  scopedPlaceholder: string;
  scopedLoading: boolean;
  uploadBusy: boolean;
  uploadReady: boolean;
  canManage: boolean;
  selectedCount: number;
  retryableSelectedCount: number;
  retrySelectedBusy: boolean;
  onUp: () => void;
  onBreadcrumb: (folderId: string | null) => void;
  onStatusFilter: (status: LibraryIngestStatus | null) => void;
  onSearchMode: (mode: LibrarySearchMode) => void;
  onResourceQuery: (value: string) => void;
  onScopedQuery: (value: string) => void;
  onScopedSearch: () => void;
  onUpload: () => void;
  onCreateFolder: () => void;
  onCreateText: () => void;
  onCreateSource: () => void;
  onCreateChildGroup: () => void;
  onCreateGitRepository: () => void;
  onRetrySelected: () => void;
  onDeleteSelected: () => void;
}) {
  const { t } = useTranslation();
  const showTrail = trail.length > 1;

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5 border-b pb-2">
      {showTrail ? (
        <>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            data-testid="library-go-up"
            aria-label={t("library.goUp")}
            title={t("library.goUp")}
            onClick={onUp}
          >
            <ArrowUp aria-hidden="true" />
          </Button>
          <nav aria-label={t("library.navigationTitle")} className="flex min-w-0 flex-wrap items-center gap-1">
            {trail.map((crumb, index) => (
              <span key={`${crumb.id ?? "root"}-${index}`} className="flex min-w-0 items-center gap-1">
                {index > 0 ? <span aria-hidden="true" className="text-muted-foreground">/</span> : null}
                <Button
                  type="button"
                  variant="ghost"
                  className={cn("max-w-40 truncate px-1.5 font-normal", index === trail.length - 1 && "font-medium")}
                  aria-current={index === trail.length - 1 ? "page" : undefined}
                  onClick={() => onBreadcrumb(crumb.id)}
                >
                  <span className="truncate">{crumb.name}</span>
                </Button>
              </span>
            ))}
          </nav>
        </>
      ) : null}

      <Select
        value={searchMode}
        onValueChange={(value) => onSearchMode(value as LibrarySearchMode)}
      >
        <SelectTrigger
          className="w-32"
          data-testid="library-search-mode"
          aria-label={t("library.searchModeLabel")}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="resources">{t("library.searchModeResources")}</SelectItem>
          <SelectItem value="body">{t("library.searchModeBody")}</SelectItem>
        </SelectContent>
      </Select>

      <div className="relative w-full min-w-0 sm:w-56 sm:flex-none sm:max-w-56">
        {searchMode === "body" ? (
          <ScanText
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
        ) : (
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
        )}
        <Input
          className="pl-8"
          value={searchMode === "body" ? scopedQuery : resourceQuery}
          aria-label={searchMode === "body" ? scopedPlaceholder : t("library.filterResourcesPlaceholder")}
          placeholder={searchMode === "body" ? scopedPlaceholder : t("library.filterResourcesPlaceholder")}
          aria-busy={searchMode === "body" && scopedLoading ? true : undefined}
          data-testid={searchMode === "body" ? "scoped-content-search-input" : "library-resource-filter"}
          data-search-mode={searchMode}
          onChange={(event) => {
            if (searchMode === "body") {
              onScopedQuery(event.target.value);
            } else {
              onResourceQuery(event.target.value);
            }
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && searchMode === "body") {
              event.preventDefault();
              onScopedSearch();
            }
          }}
        />
      </div>

      <Select
        value={statusFilter ?? "__all__"}
        onValueChange={(value) => onStatusFilter(value === "__all__" ? null : value as LibraryIngestStatus)}
      >
        <SelectTrigger
          className="w-36"
          data-testid="library-status-filter"
          aria-label={t("library.filterByStatus")}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__all__">{t("library.allStatuses")}</SelectItem>
          <SelectItem value="succeeded">{libraryStatusLabel(t, "succeeded")}</SelectItem>
          <SelectItem value="failed">{libraryStatusLabel(t, "failed")}</SelectItem>
        </SelectContent>
      </Select>

      {selectedCount > 0 ? (
        <>
          <Button
            type="button"
            variant="outline"
            data-testid="library-retry-selected"
            disabled={retryableSelectedCount === 0 || retrySelectedBusy}
            aria-busy={retrySelectedBusy}
            title={retryableSelectedCount === 0 ? t("library.retrySelectedUnavailable") : undefined}
            onClick={onRetrySelected}
          >
            <RotateCcw aria-hidden="true" />
            {t("library.retrySelected")}
            {retryableSelectedCount > 0 ? ` (${retryableSelectedCount})` : ""}
          </Button>
          <Button
            type="button"
            variant="destructive"
            data-testid="library-delete-selected"
            onClick={onDeleteSelected}
          >
            {t("library.deleteSelected")}
            {` (${selectedCount})`}
          </Button>
        </>
      ) : null}

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" variant="outline" data-testid="library-create-menu">
            <Plus aria-hidden="true" />
            {t("common.new")}
            <ChevronDown aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={onCreateChildGroup}>{t("groups.createChild")}</DropdownMenuItem>
          <DropdownMenuItem onSelect={onCreateFolder}>{t("library.newFolder")}</DropdownMenuItem>
          <DropdownMenuItem onSelect={onCreateText}>{t("library.newTextFile")}</DropdownMenuItem>
          <DropdownMenuItem onSelect={onCreateSource}>{t("library.newSourceFolder")}</DropdownMenuItem>
          {canManage ? (
            <DropdownMenuItem onSelect={onCreateGitRepository}>{t("groups.gitSources.newRepository")}</DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      <Button
        type="button"
        data-testid="library-upload"
        disabled={uploadBusy || !uploadReady}
        aria-busy={uploadBusy}
        title={uploadReady ? undefined : t("library.sourcePolicyUnavailable")}
        onClick={onUpload}
      >
        <Upload aria-hidden="true" />
        <span className="hidden sm:inline">{t("common.upload")}</span>
      </Button>
    </div>
  );
}
