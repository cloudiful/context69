import { useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearchParams } from "react-router";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { buildSearchReturnLocation, isSearchReturn } from "@/utils/search-target";
import { loadSearchSession } from "@/utils/search";

import {
  CreateFolderDialog,
  CreateTextFileDialog,
  MoveEntryDialog,
  PreviewDialog,
  ScopedSearchDialog,
  SourceFolderDialog,
} from "@/features/library/library-dialogs";
import { LibraryTable } from "@/features/library/library-table";
import { LibraryToolbar } from "@/features/library/library-toolbar";
import { buildGroupExplorerEntry, canReleaseFileSource, type ExplorerEntry } from "@/features/library/library-model";
import { useLibraryWorkspace } from "@/features/library/use-library-workspace";

import { useGroupWorkspaceContext } from "./group-workspace";

export function GroupOverviewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const state = useGroupWorkspaceContext();
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const library = useLibraryWorkspace({
    groupPath: state.groupPath,
    groupName: state.group?.name ?? null,
  });

  const fromSearch = isSearchReturn(new URLSearchParams(searchParams.toString()));

  const groupEntries = useMemo(
    () => state.children.map(buildGroupExplorerEntry),
    [state.children],
  );

  const trail = useMemo(
    () => library.selectedFolderTrail.map((folder) => ({ id: folder.folder_id ?? null, name: folder.name })),
    [library.selectedFolderTrail],
  );

  function goBackToSearch() {
    if (window.history.length > 1) {
      navigate(-1);
      return;
    }
    const session = loadSearchSession();
    if (session?.filters.query) {
      navigate(buildSearchReturnLocation(session.filters, session.page));
      return;
    }
    navigate("/search");
  }

  const selectedCount = library.selectedKeys.size;
  const selectedFile = library.selectedFileEntry;
  const selectedFileId = selectedFile?.id ?? null;

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden" data-testid="group-overview">
      {fromSearch ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-testid="group-back-to-search"
            onClick={goBackToSearch}
          >
            {t("document.backToSearch")}
          </Button>
        </div>
      ) : null}

      <LibraryToolbar
        trail={trail}
        groupName={state.group?.name ?? null}
        canGoUp={library.canGoUp}
        countLabel={library.resourcesPagination
          ? t("library.resourceCount", { count: library.resourcesPagination.total })
          : ""}
        statusFilter={library.statusFilter}
        resourceQuery={library.query}
        scopedQuery={library.scopedSearch.query}
        scopedPlaceholder={t("search.scoped.placeholder", {
          folder: library.selectedFolder?.name ?? t("library.rootFolder"),
        })}
        scopedLoading={library.scopedSearch.loading}
        retryAllCount={library.retryAllFailedCount}
        retryAllBusy={library.retryAllBusy}
        uploadBusy={library.uploadBusy}
        releaseAfterUpload={library.releaseSourceAfterUpload}
        selectedCount={selectedCount}
        onUp={() => library.navigateToFolder(library.parentFolderId)}
        onBreadcrumb={(folderId) => library.navigateToFolder(folderId)}
        onStatusFilter={library.changeStatusFilter}
        onResourceQuery={library.changeQuery}
        onScopedQuery={library.setScopedQuery}
        onScopedSearch={() => void library.runScopedSearch()}
        onRetryAll={library.retryAllFailed}
        onUpload={() => fileInputRef.current?.click()}
        onReleaseAfterUpload={library.setReleaseSourceAfterUpload}
        onCreateFolder={() => library.setCreateFolderOpen(true)}
        onCreateText={() => library.setCreateTextOpen(true)}
        onCreateSource={library.openSourceFolderCreate}
        onCreateChildGroup={state.openCreateChild}
        onDeleteSelected={library.deleteSelected}
      />
      <input
        ref={fileInputRef}
        type="file"
        multiple
        accept=".pdf,.docx,.xlsx,.md,.txt"
        className="sr-only"
        data-testid="library-upload-input"
        onChange={(event) => {
          const files = Array.from(event.target.files ?? []);
          event.target.value = "";
          void library.uploadFiles(files);
        }}
      />

      {groupEntries.length > 0 ? (
        <div className="relative w-full max-w-64">
          <Input
            className="h-8"
            value={state.childSearch}
            aria-label={t("groups.searchChildren")}
            placeholder={t("groups.searchChildren")}
            data-testid="child-group-search"
            onChange={(event) => state.setChildSearch(event.target.value)}
          />
        </div>
      ) : null}

      <div className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        <LibraryTable
          groupPath={state.groupPath}
          groupEntries={groupEntries}
          childPage={state.childPage ? { items: state.children, pagination: state.childPage } : null}
          entries={library.entries}
          loading={library.loading}
          error={library.error}
          pagination={library.resourcesPagination}
          selectedFolderId={library.folderId}
          expandedKeys={library.expandedKeys}
          selectedKeys={library.selectedKeys}
          unavailableFileIds={library.unavailableFileIds}
          retryingFileIds={library.retryingFileIds}
          releasingFileIds={library.releasingFileIds}
          sortBy={library.sortBy}
          sortDirection={library.sortDirection}
          resourceQuery={library.query}
          statusFilter={library.statusFilter}
          onToggleSelect={library.toggleSelect}
          onToggleFolder={library.toggleFolderExpansion}
          onOpenEntry={library.openEntry}
          onPreviewEntry={(entry: ExplorerEntry) => {
            if (entry.kind === "file") {
              library.selectFile(entry.id);
            }
          }}
          onEditSource={(entry) => {
            if (entry.kind === "file") {
              void library.openSourceFolderEditor(entry);
            }
          }}
          onMoveEntry={library.requestMoveEntry}
          onDeleteEntry={library.deleteEntry}
          onSyncSource={(folderId) => void library.syncSourceFolder(folderId)}
          onReleaseSource={library.releaseFileSource}
          onRetryFile={(fileId) => void library.retryFile(fileId)}
          onRefresh={() => void library.refresh()}
          onSort={library.changeSort}
          onPage={library.changePage}
          onPageSize={library.changePageSize}
          onOpenGroup={(group) => navigate(`/groups/${encodeURIComponent(group.group_path ?? group.group_key)}`)}
          onEditGroup={state.openEditChild}
          onMoveGroup={state.openMoveGroup}
          onDeleteGroup={state.requestDeleteChild}
          onChildPage={state.setChildPage}
          onChildPageSize={state.setPageSize}
          canRetryFile={library.canRetryFile}
          canRelease={library.canRelease}
        />
      </div>

      <CreateFolderDialog
        open={library.createFolderOpen}
        busy={library.createBusy}
        parentName={library.selectedFolder?.path ?? state.groupPath}
        onOpenChange={library.setCreateFolderOpen}
        onSubmit={(name) => void library.createFolder(name)}
      />
      <CreateTextFileDialog
        open={library.createTextOpen}
        busy={library.createBusy}
        parentName={library.selectedFolder?.path ?? state.groupPath}
        onOpenChange={library.setCreateTextOpen}
        onSubmit={(payload) => void library.createTextFile(payload)}
      />
      <MoveEntryDialog
        state={library.moveDialog}
        busy={library.actionBusy}
        onOpenChange={(open) => {
          if (!open) {
            library.setMoveDialogState(null);
          }
        }}
        onSubmit={(target) => void library.confirmMove(target)}
      />
      <SourceFolderDialog
        state={library.sourceFolder}
        onOpenChange={(open) => {
          if (!open) {
            library.closeSourceFolder();
          }
        }}
        onChangeValue={(value) => library.updateSourceFolderValue(value)}
        onChangeFolderName={(value) => library.updateSourceFolderName(value)}
        onSubmit={() => void library.saveSourceFolder({ folderName: library.sourceFolder.folderName, value: library.sourceFolder.value })}
      />
      <PreviewDialog
        open={!!selectedFileId}
        title={selectedFile?.name ?? ""}
        detail={library.detail}
        loading={library.detailLoading}
        error={library.detailError}
        canRetry={!!selectedFile && library.canRetryFile(selectedFile)}
        retrying={!!selectedFile && library.retryingFileIds.includes(selectedFile.id)}
        canRelease={!!selectedFile && canReleaseFileSource(selectedFile)}
        releasing={!!selectedFile && library.releasingFileIds.includes(selectedFile.id)}
        onOpenChange={(open) => {
          if (!open) {
            library.closePreview();
          }
        }}
        onRetry={() => {
          if (selectedFile) {
            void library.retryFile(selectedFile.id);
          }
        }}
        onRelease={() => {
          if (selectedFile) {
            library.releaseFileSource(selectedFile.id, selectedFile.name);
          }
        }}
      />
      <ScopedSearchDialog
        open={library.scopedSearch.open}
        query={library.scopedSearch.query}
        folderName={library.selectedFolder?.name ?? t("library.rootFolder")}
        loading={library.scopedSearch.loading}
        results={library.scopedSearch.results}
        onOpenChange={library.setScopedOpen}
        onOpenResult={(hit) => {
          if (hit.library_file_id) {
            library.setScopedOpen(false);
            library.selectFile(hit.library_file_id);
          }
        }}
      />
      <ConfirmDialog
        open={!!library.confirmRequest}
        title={library.confirmRequest?.title ?? ""}
        message={library.confirmRequest?.message ?? ""}
        acceptLabel={library.confirmRequest?.acceptLabel}
        onAccept={() => library.confirmRequest?.action()}
        onCancel={() => library.setConfirmRequest(null)}
      />
    </div>
  );
}
