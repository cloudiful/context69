import { ArrowUp, ChevronDown, Plus, RotateCcw, ScanText, Search, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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

import { libraryStatusLabel } from "./library-model";

export function LibraryToolbar({
  trail,
  groupName,
  canGoUp,
  countLabel,
  statusFilter,
  resourceQuery,
  scopedQuery,
  scopedPlaceholder,
  scopedLoading,
  retryAllCount,
  retryAllBusy,
  uploadBusy,
  releaseAfterUpload,
  selectedCount,
  onUp,
  onBreadcrumb,
  onStatusFilter,
  onResourceQuery,
  onScopedQuery,
  onScopedSearch,
  onRetryAll,
  onUpload,
  onReleaseAfterUpload,
  onCreateFolder,
  onCreateText,
  onCreateSource,
  onCreateChildGroup,
  onDeleteSelected,
}: {
  trail: Array<{ id: string | null; name: string }>;
  groupName: string | null;
  canGoUp: boolean;
  countLabel: string;
  statusFilter: LibraryIngestStatus | null;
  resourceQuery: string;
  scopedQuery: string;
  scopedPlaceholder: string;
  scopedLoading: boolean;
  retryAllCount: number;
  retryAllBusy: boolean;
  uploadBusy: boolean;
  releaseAfterUpload: boolean;
  selectedCount: number;
  onUp: () => void;
  onBreadcrumb: (folderId: string | null) => void;
  onStatusFilter: (status: LibraryIngestStatus | null) => void;
  onResourceQuery: (value: string) => void;
  onScopedQuery: (value: string) => void;
  onScopedSearch: () => void;
  onRetryAll: () => void;
  onUpload: () => void;
  onReleaseAfterUpload: (value: boolean) => void;
  onCreateFolder: () => void;
  onCreateText: () => void;
  onCreateSource: () => void;
  onCreateChildGroup: () => void;
  onDeleteSelected: () => void;
}) {
  const { t } = useTranslation();
  const homeLabel = groupName ?? t("library.rootFolder");
  const crumbs = [
    { id: null as string | null, name: homeLabel },
    ...trail.slice(1).map((folder) => ({ id: folder.id, name: folder.name })),
  ];

  return (
    <div className="grid min-w-0 gap-2 border-b pb-2">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          data-testid="library-go-up"
          aria-label={t("library.goUp")}
          title={t("library.goUp")}
          disabled={!canGoUp}
          onClick={onUp}
        >
          <ArrowUp aria-hidden="true" />
        </Button>
        <nav aria-label={t("library.navigationTitle")} className="flex min-w-0 flex-wrap items-center gap-1">
          {crumbs.map((crumb, index) => (
            <span key={`${crumb.id ?? "root"}-${index}`} className="flex min-w-0 items-center gap-1">
              {index > 0 ? <span aria-hidden="true" className="text-muted-foreground">/</span> : null}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className={cn("max-w-48 truncate px-1.5 font-normal", index === crumbs.length - 1 && "font-medium")}
                aria-current={index === crumbs.length - 1 ? "page" : undefined}
                onClick={() => onBreadcrumb(crumb.id)}
              >
                <span className="truncate">{crumb.name}</span>
              </Button>
            </span>
          ))}
        </nav>
        <span className="text-xs text-muted-foreground">{countLabel}</span>
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <div className="relative w-full min-w-0 sm:w-56 sm:flex-none sm:max-w-56">
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            className="pl-8"
            value={resourceQuery}
            aria-label={t("library.filterResourcesPlaceholder")}
            placeholder={t("library.filterResourcesPlaceholder")}
            data-testid="library-resource-filter"
            onChange={(event) => onResourceQuery(event.target.value)}
          />
        </div>

        <Select
          value={statusFilter ?? "__all__"}
          onValueChange={(value) => onStatusFilter(value === "__all__" ? null : value as LibraryIngestStatus)}
        >
          <SelectTrigger
            className="w-40"
            size="sm"
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

        <div className="relative w-full min-w-0 sm:w-48 sm:flex-none">
          <ScanText
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            className="pl-8"
            value={scopedQuery}
            aria-label={scopedPlaceholder}
            placeholder={scopedPlaceholder}
            data-testid="scoped-content-search-input"
            onChange={(event) => onScopedQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                onScopedSearch();
              }
            }}
          />
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-testid="scoped-content-search-trigger"
          disabled={scopedLoading || !scopedQuery.trim()}
          onClick={onScopedSearch}
        >
          <ScanText aria-hidden="true" />
          {t("search.scoped.run")}
        </Button>

        <Button
          type="button"
          variant="outline"
          size="sm"
          data-testid="retry-all-failed"
          disabled={retryAllBusy || retryAllCount === 0}
          aria-busy={retryAllBusy}
          title={t("library.retryAllFailed")}
          onClick={onRetryAll}
        >
          <RotateCcw aria-hidden="true" />
          <span className="hidden md:inline">{t("library.retryAllFailed")}</span>
        </Button>

        <Button
          type="button"
          variant="destructive"
          size="sm"
          data-testid="library-delete-selected"
          disabled={selectedCount === 0}
          onClick={onDeleteSelected}
        >
          {t("library.deleteSelected")}
          {selectedCount > 0 ? ` (${selectedCount})` : ""}
        </Button>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button type="button" variant="outline" size="sm" data-testid="library-create-menu">
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
          </DropdownMenuContent>
        </DropdownMenu>

        <Button
          type="button"
          size="sm"
          data-testid="library-upload"
          disabled={uploadBusy}
          aria-busy={uploadBusy}
          onClick={onUpload}
        >
          <Upload aria-hidden="true" />
          <span className="hidden sm:inline">{t("common.upload")}</span>
        </Button>
      </div>

      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        <Checkbox
          checked={releaseAfterUpload}
          onCheckedChange={(checked) => onReleaseAfterUpload(checked === true)}
          data-testid="release-source-after-upload"
        />
        <span>{t("library.releaseSourceAfterUpload")}</span>
        <span className="sr-only">{t("library.releaseSourceAfterUploadHint")}</span>
      </label>
    </div>
  );
}
