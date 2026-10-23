import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type {
  LibraryFileDetailResponse,
  LibraryIngestStatus,
  LibraryResourceItem,
  LibraryResourceSortBy,
  LibraryTreeResponse,
  SearchHit,
  SortDirection,
  SourceConfigInput,
} from "@/services/api";

import type { TaskRef, TaskResponse } from "@/services/api";
import {
  buildLibraryFolderQuery,
  buildExplorerEntries,
  canMutateEntry,
  canReleaseFileSource,
  collectDescendantFolderIds,
  fileResourceEntry,
  findFileLocation,
  findFolderById,
  findFolderTrail,
  flattenFolderOptions,
  folderKey,
  folderResourceEntry,
  getParentFolderId,
  isRetryableFileStatus,
  isValidLibraryFolder,
  readLibraryFileQuery,
  readLibraryFolderQuery,
  type ExplorerEntry,
  type FileExplorerEntry,
  type FolderSummary,
} from "./library-model";
import { settleTaskRefs } from "./library-tasks";

export const DEFAULT_LIBRARY_PAGE_SIZE = 50;
const FAILED_COLLECT_PAGE_SIZE = 100;

export interface ConfirmRequest {
  title: string;
  message: string;
  acceptLabel: string;
  action: () => void;
}

export interface MoveDialogState {
  entry: ExplorerEntry;
  options: Array<{ label: string; value: string | null }>;
}

export interface SourceFolderState {
  open: boolean;
  busy: boolean;
  folderId: string | null;
  folderName: string;
  title: string;
  value: string;
}

export interface ScopedSearchState {
  query: string;
  loading: boolean;
  open: boolean;
  results: SearchHit[];
}

interface UseLibraryWorkspaceOptions {
  groupPath: string;
  groupName: string | null;
}

function defaultSourceTemplate(): string {
  return JSON.stringify({
    source_key: "",
    display_name: "",
    description: "",
    example_queries: [],
    connection: "",
    sync_strategy: "cursor",
    connector_type: "postgres_sql",
    base_query: "",
    batch_size: 200,
  }, null, 2);
}

function folderIdFromEntry(entry: ExplorerEntry): string | null {
  return entry.parentFolderId;
}

let ownerSequence = 0;

interface TaskOwner {
  readonly id: number;
  readonly controller: AbortController;
  readonly signal: AbortSignal;
  disposed: boolean;
}

function createTaskOwner(): TaskOwner {
  ownerSequence += 1;
  const controller = new AbortController();
  return { id: ownerSequence, controller, signal: controller.signal, disposed: false };
}

function disposeTaskOwner(owner: TaskOwner) {
  owner.disposed = true;
  owner.controller.abort();
}

interface TaskOperation {
  /** Aborts the admission and the settlement of this operation. */
  readonly signal: AbortSignal;
  /** Whether the owning workspace/group this operation started in is gone. */
  isDisposed: () => boolean;
  /** Settles the accepted tasks under this operation's lifetime. */
  settle: (taskRefs: TaskRef[]) => Promise<TaskResponse[]>;
  /** Releases the operation's listeners; always called in `finally`. */
  finish: () => void;
}

/**
 * Owns the group library: the folder tree, the paged resource window, the
 * URL-selected folder/file, the file detail preview, uploads, folder/file
 * mutations, source lifecycle actions, bulk retry and multi-selection deletion.
 *
 * Every request key carries the actual group path and folder/page/status/sort,
 * so switching group or folder fences stale responses; mutations never retry
 * and only invalidate library-owned keys.
 */
export function useLibraryWorkspace({ groupPath, groupName }: UseLibraryWorkspaceOptions) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const searchString = searchParams.toString();

  const folderId = readLibraryFolderQuery(new URLSearchParams(searchString));
  const selectedFileId = readLibraryFileQuery(new URLSearchParams(searchString));

  // The paged window and the row selection belong to one folder scope. Entering
  // another folder (tree, up, breadcrumb, deep link, back/forward) therefore
  // starts at page one with no rows selected, and the first request of the new
  // folder already carries that page instead of briefly reading the old offset.
  const resourceScope = `${groupPath}\u0000${folderId ?? ""}`;
  const [pagingWindow, setPagingWindow] = useState({ scope: resourceScope, page: 1 });
  if (pagingWindow.scope !== resourceScope) {
    // Commit the transition: the new scope owns page one, so a later visit to a
    // previous scope cannot resume that scope's abandoned page either.
    setPagingWindow({ scope: resourceScope, page: 1 });
  }
  const page = pagingWindow.scope === resourceScope ? pagingWindow.page : 1;
  const setPage = useCallback((nextPage: number) => {
    setPagingWindow({ scope: resourceScope, page: nextPage });
  }, [resourceScope]);
  const [pageSize, setPageSize] = useState(DEFAULT_LIBRARY_PAGE_SIZE);
  const [query, setQueryState] = useState("");
  const [statusFilter, setStatusFilter] = useState<LibraryIngestStatus | null>(null);
  const [sortBy, setSortBy] = useState<LibraryResourceSortBy>("updated_at");
  const [sortDirection, setSortDirection] = useState<SortDirection>("desc");
  const [expandedKeys, setExpandedKeys] = useState<Record<string, boolean>>({ [folderKey(null)]: true });
  const [selectionScope, setSelectionScope] = useState(resourceScope);
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [resourceContextKey, setResourceContextKey] = useState<string | null>(null);
  if (selectionScope !== resourceScope) {
    setSelectionScope(resourceScope);
    setSelectedKeys(new Set());
    setResourceContextKey(null);
  }
  const [retryingFileIds, setRetryingFileIds] = useState<string[]>([]);
  const [releasingFileIds, setReleasingFileIds] = useState<string[]>([]);
  const [unavailableFileIds, setUnavailableFileIds] = useState<string[]>([]);
  const [uploadBusy, setUploadBusy] = useState(false);
  const [actionBusy, setActionBusy] = useState(false);
  const [releaseSourceAfterUpload, setReleaseSourceAfterUpload] = useState(false);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [createFolderOpen, setCreateFolderOpen] = useState(false);
  const [createTextOpen, setCreateTextOpen] = useState(false);
  const [createBusy, setCreateBusy] = useState(false);
  const [moveDialog, setMoveDialog] = useState<MoveDialogState | null>(null);
  const [sourceFolder, setSourceFolder] = useState<SourceFolderState>({
    open: false,
    busy: false,
    folderId: null,
    folderName: "",
    title: "",
    value: defaultSourceTemplate(),
  });
  const [scopedSearch, setScopedSearch] = useState<ScopedSearchState>({
    query: "",
    loading: false,
    open: false,
    results: [],
  });
  const scopedSearchController = useRef<AbortController | null>(null);
  const searchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // One owner lifetime per mounted, current-group workspace. Every library task
  // observation registers here, so unmounting or switching group aborts pending
  // admissions and settlements instead of polling for a scope that is gone. The
  // server-side task is never cancelled: only local observation stops.
  const currentGroupPathRef = useRef(groupPath);
  currentGroupPathRef.current = groupPath;
  const ownerRef = useRef<TaskOwner | null>(null);
  if (!ownerRef.current) {
    ownerRef.current = createTaskOwner();
  }
  const operationControllers = useRef<Set<AbortController>>(new Set());

  const treeQuery = useQuery({
    queryKey: ["library", "tree", groupPath],
    queryFn: ({ signal }) => apiClient.getGroupLibraryTree(groupPath, { signal }),
    retry: false,
  });

  const resourcesQuery = useQuery({
    queryKey: [
      "library",
      "resources",
      groupPath,
      folderId ?? "root",
      page,
      pageSize,
      query,
      statusFilter ?? "",
      sortBy,
      sortDirection,
    ],
    queryFn: ({ signal }) => apiClient.getGroupLibraryResources(groupPath, {
      folderId,
      page,
      pageSize,
      query,
      status: statusFilter,
      sortBy,
      sortDirection,
    }, { signal }),
    enabled: !!treeQuery.data,
    retry: false,
    placeholderData: (previous) => previous,
  });

  const detailQuery = useQuery({
    queryKey: ["library", "file", groupPath, selectedFileId],
    queryFn: ({ signal }) => apiClient.getGroupLibraryFile(groupPath, selectedFileId as string, { signal }),
    enabled: !!selectedFileId,
    retry: false,
  });

  const failedCountQuery = useQuery({
    queryKey: ["library", "failed-count", groupPath, folderId ?? "root"],
    queryFn: ({ signal }) => apiClient.getGroupLibraryResources(groupPath, {
      folderId,
      recursive: true,
      page: 1,
      pageSize: 1,
      query: "",
      status: "failed",
      sortBy: "updated_at",
      sortDirection: "desc",
    }, { signal }),
    enabled: !!treeQuery.data,
    retry: false,
  });

  const tree: LibraryTreeResponse | null = treeQuery.data ?? null;
  const root = tree?.root ?? null;

  const selectedFolder = useMemo(() => {
    if (!root || folderId === null) {
      return root;
    }
    return findFolderById(root, folderId) ?? root;
  }, [root, folderId]);

  const selectedFolderTrail = useMemo(() => {
    if (!root) {
      return [];
    }
    return findFolderTrail(root, folderId) ?? [root];
  }, [root, folderId]);

  const entries = useMemo<ExplorerEntry[]>(() => {
    if (!selectedFolder || !resourcesQuery.data) {
      return [];
    }
    return resourcesQuery.data.items.map((item: LibraryResourceItem) => (
      item.kind === "folder"
        ? folderResourceEntry(item, selectedFolder)
        : fileResourceEntry(item, selectedFolder)
    ));
  }, [resourcesQuery.data, selectedFolder]);

  const explorerEntries = useMemo(() => (
    root ? buildExplorerEntries(root, expandedKeys) : []
  ), [expandedKeys, root]);

  const selectedExplorerEntry = useMemo(() => {
    if (selectedFileId) {
      return explorerEntries.find((entry) => entry.kind === "file" && entry.id === selectedFileId) ?? null;
    }
    if (folderId) {
      const folderEntry = explorerEntries.find((entry) => entry.kind === "folder" && entry.id === folderId);
      if (folderEntry) {
        return folderEntry;
      }
    }
    return null;
  }, [explorerEntries, folderId, selectedFileId]);

  const selectedFileEntry = selectedFileId
    ? explorerEntries.find((entry): entry is FileExplorerEntry => (
      entry.kind === "file" && entry.id === selectedFileId
    )) ?? null
    : null;

  const selectedFolderSummary = useMemo<FolderSummary | null>(() => {
    if (!selectedFolder) {
      return null;
    }
    return {
      name: selectedFolder.name,
      path: selectedFolder.path,
      childFolderCount: selectedFolder.children.length,
      fileCount: selectedFolder.files.length,
      isSourceFolder: selectedFolder.files.some((file) => file.filename.toLowerCase() === "source.json"),
      isSourceRecordsFolder: selectedFolder.name === "records" && !!selectedFolder.parent_folder_id,
      processingCount: selectedFolder.processing_count,
    };
  }, [selectedFolder]);

  const moveOptions = useMemo(
    () => (root ? flattenFolderOptions(root) : []),
    [root],
  );

  const canGoUp = selectedFolderTrail.length > 1;

  const detail: LibraryFileDetailResponse | null = detailQuery.data ?? null;

  const observeSourceAvailability = useCallback((fileId: string, available: boolean) => {
    setUnavailableFileIds((current) => (
      available
        ? current.filter((id) => id !== fileId)
        : [...new Set([...current, fileId])]
    ));
  }, []);

  useEffect(() => {
    if (detail?.source_available === false) {
      observeSourceAvailability(detail.file_id, false);
    } else if (detail?.source_available === true) {
      observeSourceAvailability(detail.file_id, true);
    }
  }, [detail, observeSourceAvailability]);

  const invalidateLibrary = useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["library", "tree", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["library", "resources", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["library", "failed-count", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["library", "file", groupPath] }),
    ]);
  }, [groupPath, queryClient]);

  const setUrlSelection = useCallback((nextFolderId: string | null, fileId: string | null) => {
    const params = new URLSearchParams(searchString);
    if (nextFolderId === null) {
      params.delete("folder");
    } else if (nextFolderId) {
      params.set("folder", nextFolderId);
    }
    if (fileId === null) {
      params.delete("file");
    } else {
      params.set("file", fileId);
    }
    const nextSearch = params.toString();
    if (nextSearch !== searchString) {
      setSearchParams(params, { replace: true });
    }
  }, [searchString, setSearchParams]);

  const navigateToFolder = useCallback((nextFolderId: string | null) => {
    setSelectedKeys(new Set());
    setUrlSelection(nextFolderId, null);
  }, [setUrlSelection]);

  const openEntry = useCallback((entry: ExplorerEntry) => {
    if (entry.kind === "folder") {
      setSelectedKeys(new Set());
      setUrlSelection(entry.id, null);
      return;
    }
    setUrlSelection(folderIdFromEntry(entry), entry.id);
  }, [setUrlSelection]);

  const selectFile = useCallback((fileId: string) => {
    setUrlSelection(folderId, fileId);
  }, [folderId, setUrlSelection]);

  const closePreview = useCallback(() => {
    setUrlSelection(folderId, null);
  }, [folderId, setUrlSelection]);

  // Correct an invalid deep link once the tree is available.
  useEffect(() => {
    if (!root) {
      return;
    }
    if (!isValidLibraryFolder(root, folderId)) {
      const params = new URLSearchParams(searchString);
      params.delete("folder");
      params.delete("file");
      setSearchParams(params, { replace: true });
      return;
    }
    if (selectedFileId) {
      const location = findFileLocation(root, selectedFileId);
      if (!location) {
        setUrlSelection(folderId, null);
        return;
      }
      if ((location.folder.folder_id ?? null) !== (folderId ?? null)) {
        setUrlSelection(location.folder.folder_id ?? null, selectedFileId);
      }
    }
  }, [folderId, root, searchString, selectedFileId, setSearchParams, setUrlSelection]);

  // Keep the trail expanded whenever the selected folder changes.
  useEffect(() => {
    if (selectedFolderTrail.length === 0) {
      return;
    }
    setExpandedKeys((current) => {
      let changed = false;
      const next = { ...current };
      for (const folder of selectedFolderTrail) {
        const key = folderKey(folder.folder_id ?? null);
        if (!next[key]) {
          next[key] = true;
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [selectedFolderTrail]);

  // A group switch starts a fresh selection and paging window.
  const previousGroup = useRef(groupPath);
  useEffect(() => {
    if (previousGroup.current === groupPath) {
      return;
    }
    previousGroup.current = groupPath;
    setQueryState("");
    setStatusFilter(null);
    setExpandedKeys({ [folderKey(null)]: true });
  }, [groupPath]);

  const toggleFolderExpansion = useCallback((id: string | null) => {
    const key = folderKey(id);
    setExpandedKeys((current) => {
      const next = { ...current };
      if (next[key]) {
        delete next[key];
      } else {
        next[key] = true;
      }
      return next;
    });
  }, []);

  const changeQuery = useCallback((value: string) => {
    setQueryState(value);
    if (searchTimer.current) {
      clearTimeout(searchTimer.current);
    }
    searchTimer.current = setTimeout(() => {
      setPage(1);
    }, 250);
  }, [setPage]);

  // A group switch is a new owner; the previous lifetime is disposed first so
  // its settlement results, toasts and window writes cannot reach the new group.
  useEffect(() => {
    const owner = createTaskOwner();
    const previous = ownerRef.current;
    ownerRef.current = owner;
    if (previous) {
      disposeTaskOwner(previous);
    }
    return () => {
      disposeTaskOwner(owner);
    };
  }, [groupPath]);

  useEffect(() => () => {
    if (searchTimer.current) {
      clearTimeout(searchTimer.current);
    }
    scopedSearchController.current?.abort();
    for (const controller of operationControllers.current) {
      controller.abort();
    }
    operationControllers.current.clear();
  }, []);

  /**
   * Starts one library operation inside the current owner lifetime. The
   * controller exists before the first awaited admission, so a task reference
   * that arrives after unmount or a group switch can never begin observation.
   */
  const beginTaskOperation = useCallback((): TaskOperation => {
    const owner = ownerRef.current as TaskOwner;
    const group = groupPath;
    const controller = new AbortController();
    const abort = () => controller.abort();
    if (owner.disposed) {
      controller.abort();
    } else {
      owner.signal.addEventListener("abort", abort, { once: true });
    }
    operationControllers.current.add(controller);

    const isDisposed = () => owner.disposed
      || ownerRef.current !== owner
      || currentGroupPathRef.current !== group;

    return {
      signal: controller.signal,
      isDisposed,
      settle: async (taskRefs) => {
        if (isDisposed() || controller.signal.aborted) {
          return [];
        }
        return settleTaskRefs(taskRefs, { signal: controller.signal });
      },
      finish: () => {
        owner.signal.removeEventListener("abort", abort);
        operationControllers.current.delete(controller);
      },
    };
  }, [groupPath]);

  const changePage = useCallback((nextPage: number) => {
    setPage(nextPage);
  }, [setPage]);

  const changePageSize = useCallback((nextSize: number) => {
    setPageSize(nextSize);
    setPage(1);
  }, [setPage]);

  const changeStatusFilter = useCallback((status: LibraryIngestStatus | null) => {
    setStatusFilter(status);
    setPage(1);
  }, [setPage]);

  const changeSort = useCallback((field: LibraryResourceSortBy, direction: SortDirection) => {
    setSortBy(field);
    setSortDirection(direction);
    setPage(1);
  }, [setPage]);

  const toggleSelect = useCallback((key: string) => {
    setSelectedKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  const clearSelection = useCallback(() => setSelectedKeys(new Set()), []);

  const refresh = useCallback(async () => {
    await invalidateLibrary();
  }, [invalidateLibrary]);

  const createFolder = useCallback(async (name: string) => {
    if (!name.trim()) {
      return;
    }
    setCreateBusy(true);
    try {
      const created = await apiClient.createGroupLibraryFolder(groupPath, {
        parent_folder_id: folderId,
        name: name.trim(),
      });
      setCreateFolderOpen(false);
      await invalidateLibrary();
      setExpandedKeys((current) => ({
        ...current,
        [folderKey(folderId)]: true,
        [folderKey(created.folder_id ?? null)]: true,
      }));
      setUrlSelection(created.folder_id ?? null, null);
      toast.success(t("library.newFolder"), { description: created.name, duration: 2500 });
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("library.createFolderFailed"),
        duration: 5000,
      });
    } finally {
      setCreateBusy(false);
    }
  }, [folderId, groupPath, invalidateLibrary, setUrlSelection, t]);

  const createTextFile = useCallback(async (payload: { title: string; content: string }) => {
    if (!payload.title.trim() || !payload.content.trim()) {
      return;
    }
    setCreateBusy(true);
    const operation = beginTaskOperation();
    try {
      const task = await apiClient.upsertGroupLibraryText(groupPath, {
        title: payload.title.trim(),
        content: payload.content,
        external_id: crypto.randomUUID(),
        folder_id: folderId,
      }, { signal: operation.signal });
      if (operation.isDisposed()) {
        return;
      }
      setCreateTextOpen(false);
      const results = await operation.settle([task]);
      if (operation.isDisposed()) {
        return;
      }
      await invalidateLibrary();
      if (operation.isDisposed()) {
        return;
      }
      if (results.some((result) => result.status === "failed")) {
        toast.error(t("common.error"), { description: t("library.createTextFileFailed") });
      } else {
        toast.success(t("library.newTextFile"), { description: payload.title.trim(), duration: 2500 });
      }
    } catch (error) {
      if (operation.isDisposed()) {
        return;
      }
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("library.createTextFileFailed"),
        duration: 5000,
      });
    } finally {
      operation.finish();
      if (!operation.isDisposed()) {
        setCreateBusy(false);
      }
    }
  }, [beginTaskOperation, folderId, groupPath, invalidateLibrary, t]);

  const uploadFiles = useCallback(async (files: File[]) => {
    if (files.length === 0) {
      return;
    }
    setUploadBusy(true);
    const operation = beginTaskOperation();
    try {
      const response = await apiClient.uploadGroupLibraryFiles(groupPath, folderId, files, {
        deleteSourceAfterProcessing: releaseSourceAfterUpload,
        signal: operation.signal,
      });
      if (operation.isDisposed()) {
        return;
      }
      if (response.files.length > 0) {
        setUrlSelection(response.files[0].folder_id ?? folderId, response.files[0].file_id);
      }
      const results = await operation.settle(response.tasks ?? []);
      if (operation.isDisposed()) {
        return;
      }
      await invalidateLibrary();
      if (operation.isDisposed()) {
        return;
      }
      if (results.some((result) => result.status === "failed")) {
        toast.error(t("common.error"), { description: t("library.uploadFailed") });
      } else {
        toast.success(t("common.upload"), { description: t("library.uploadSuccess"), duration: 2500 });
      }
    } catch (error) {
      if (operation.isDisposed()) {
        return;
      }
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("library.uploadFailed"),
        duration: 5000,
      });
    } finally {
      operation.finish();
      if (!operation.isDisposed()) {
        setUploadBusy(false);
        setReleaseSourceAfterUpload(false);
      }
    }
  }, [beginTaskOperation, folderId, groupPath, invalidateLibrary, releaseSourceAfterUpload, setUrlSelection, t]);

  const requestMoveEntry = useCallback((entry: ExplorerEntry) => {
    const excluded = entry.kind === "folder"
      ? collectDescendantFolderIds(entry.folder)
      : [];
    setMoveDialog({
      entry,
      options: moveOptions.filter((option) => !excluded.includes(option.value)),
    });
  }, [moveOptions]);

  const confirmMove = useCallback(async (targetFolderId: string | null) => {
    const current = moveDialog;
    if (!current) {
      return;
    }
    setActionBusy(true);
    try {
      if (current.entry.kind === "folder") {
        if (!current.entry.id) {
          return;
        }
        await apiClient.moveGroupLibraryFolder(groupPath, current.entry.id, {
          target_folder_id: targetFolderId,
        });
      } else {
        await apiClient.moveGroupLibraryFile(groupPath, current.entry.id, {
          target_folder_id: targetFolderId,
        });
      }
      setMoveDialog(null);
      await invalidateLibrary();
      setUrlSelection(
        targetFolderId,
        current.entry.kind === "file" ? current.entry.id : null,
      );
      toast.success(t("common.move"), { description: current.entry.name, duration: 2500 });
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("library.moveFailed"),
        duration: 5000,
      });
    } finally {
      setActionBusy(false);
    }
  }, [groupPath, invalidateLibrary, moveDialog, setUrlSelection, t]);

  const deleteEntry = useCallback((entry: ExplorerEntry) => {
    setConfirmRequest({
      title: t("common.delete"),
      message: entry.kind === "folder"
        ? t("library.deleteFolderConfirm", { name: entry.name })
        : t("library.deleteFileConfirm", { name: entry.name }),
      acceptLabel: t("common.delete"),
      action: () => {
        setConfirmRequest(null);
        void (async () => {
          setActionBusy(true);
          const operation = beginTaskOperation();
          try {
            if (entry.kind === "folder" && !entry.id) {
              return;
            }
            const taskRef = entry.kind === "folder"
              ? await apiClient.deleteGroupLibraryFolder(groupPath, entry.id as string, {
                signal: operation.signal,
              })
              : await apiClient.deleteGroupLibraryFile(groupPath, entry.id, {
                signal: operation.signal,
              });
            if (operation.isDisposed()) {
              return;
            }
            if (entry.kind === "folder") {
              setUrlSelection(null, null);
            } else if (selectedFileId === entry.id) {
              setUrlSelection(folderId, null);
            }
            const settled = await operation.settle([taskRef]);
            if (operation.isDisposed()) {
              return;
            }
            await invalidateLibrary();
            if (operation.isDisposed()) {
              return;
            }
            const finished = settled.find((task) => task.task_id === taskRef?.task_id);
            if (finished?.status === "succeeded") {
              toast.success(t("common.delete"), { description: entry.name, duration: 2500 });
            } else {
              toast.error(t("common.error"), {
                description: finished?.error_summary
                  ?? (entry.kind === "folder"
                    ? t("library.deleteFolderFailed")
                    : t("library.deleteFileFailed")),
                duration: 5000,
              });
            }
          } catch (error) {
            if (operation.isDisposed()) {
              return;
            }
            await invalidateLibrary();
            toast.error(t("common.error"), {
              description: error instanceof Error
                ? error.message
                : entry.kind === "folder" ? t("library.deleteFolderFailed") : t("library.deleteFileFailed"),
              duration: 5000,
            });
          } finally {
            operation.finish();
            if (!operation.isDisposed()) {
              setActionBusy(false);
            }
          }
        })();
      },
    });
  }, [beginTaskOperation, folderId, groupPath, invalidateLibrary, selectedFileId, setUrlSelection, t]);

  const deleteSelected = useCallback(() => {
    const targets = [...selectedKeys]
      .map((key) => explorerEntries.find((entry) => entry.key === key))
      .filter((entry): entry is ExplorerEntry => !!entry && canMutateEntry(entry));
    if (targets.length === 0) {
      return;
    }
    setConfirmRequest({
      title: t("library.deleteSelected"),
      message: t("library.deleteSelectedConfirm", { count: targets.length }),
      acceptLabel: t("common.delete"),
      action: () => {
        setConfirmRequest(null);
        void (async () => {
          setActionBusy(true);
          const operation = beginTaskOperation();
          try {
            const accepted: Array<{ entry: ExplorerEntry; taskRef: TaskRef }> = [];
            const rejected: ExplorerEntry[] = [];
            for (const entry of targets) {
              if (operation.isDisposed() || operation.signal.aborted) {
                return;
              }
              try {
                const taskRef = entry.kind === "folder" && entry.id
                  ? await apiClient.deleteGroupLibraryFolder(groupPath, entry.id, {
                    signal: operation.signal,
                  })
                  : entry.kind === "file"
                    ? await apiClient.deleteGroupLibraryFile(groupPath, entry.id, {
                      signal: operation.signal,
                    })
                    : null;
                if (taskRef) {
                  accepted.push({ entry, taskRef });
                } else {
                  rejected.push(entry);
                }
              } catch {
                rejected.push(entry);
              }
            }

            // Wait for the accepted deletions to finish, so the refreshed window
            // and the report describe their real outcome instead of the intent.
            const settled = await operation.settle(accepted.map(({ taskRef }) => taskRef));
            if (operation.isDisposed()) {
              return;
            }
            const byTaskId = new Map(settled.map((task) => [task.task_id, task]));
            const unfinished = accepted.filter(({ taskRef }) => (
              byTaskId.get(taskRef.task_id)?.status !== "succeeded"
            ));
            const failedEntries = [
              ...rejected,
              ...unfinished.map(({ entry }) => entry),
            ];
            const succeededCount = targets.length - failedEntries.length;

            setSelectedKeys(new Set());
            if (targets.some((entry) => entry.kind === "folder" && entry.id === folderId)) {
              setUrlSelection(null, null);
            } else if (targets.some((entry) => entry.kind === "file" && entry.id === selectedFileId)) {
              setUrlSelection(folderId, null);
            }
            await invalidateLibrary();
            if (operation.isDisposed()) {
              return;
            }

            if (succeededCount > 0) {
              toast.success(t("common.delete"), {
                description: t("library.deleteSelectedSuccess", { count: succeededCount }),
                duration: 2500,
              });
            }
            if (failedEntries.length > 0) {
              const summary = unfinished
                .map(({ taskRef }) => byTaskId.get(taskRef.task_id)?.error_summary)
                .find((value): value is string => !!value);
              const kinds = new Set(failedEntries.map((entry) => entry.kind));
              const fallback = kinds.size === 1
                ? kinds.has("folder") ? t("library.deleteFolderFailed") : t("library.deleteFileFailed")
                : `${t("library.deleteFolderFailed")} · ${t("library.deleteFileFailed")}`;
              toast.error(t("common.error"), {
                description: summary ?? fallback,
                duration: 5000,
              });
            }
          } finally {
            operation.finish();
            if (!operation.isDisposed()) {
              setActionBusy(false);
            }
          }
        })();
      },
    });
  }, [beginTaskOperation, explorerEntries, folderId, groupPath, invalidateLibrary, selectedFileId, selectedKeys, setUrlSelection, t]);

  const retryFile = useCallback(async (fileId: string) => {
    if (retryingFileIds.includes(fileId) || unavailableFileIds.includes(fileId)) {
      return;
    }
    setRetryingFileIds((current) => [...current, fileId]);
    const operation = beginTaskOperation();
    try {
      const fileDetail = await apiClient.getGroupLibraryFile(groupPath, fileId, {
        signal: operation.signal,
      });
      if (operation.isDisposed()) {
        return;
      }
      observeSourceAvailability(fileId, fileDetail.source_available !== false);
      if (fileDetail.source_available === false) {
        throw new Error(t("library.sourceMissingMessage"));
      }
      const task = await apiClient.submitTask({
        kind: "retry_file_batch",
        group_path: groupPath,
        items: [{ file_id: fileId }],
      }, { signal: operation.signal });
      if (operation.isDisposed()) {
        return;
      }
      toast.success(t("library.retryAccepted"), { description: t("library.retryAcceptedMessage"), duration: 2500 });
      const results = await operation.settle([task]);
      if (operation.isDisposed()) {
        return;
      }
      await invalidateLibrary();
      if (operation.isDisposed()) {
        return;
      }
      if (results.some((result) => result.status === "failed")) {
        toast.error(t("common.error"), { description: t("library.retryFailed") });
      }
    } catch (error) {
      if (operation.isDisposed()) {
        return;
      }
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("library.retryFailed"),
        duration: 5000,
      });
    } finally {
      operation.finish();
      if (!operation.isDisposed()) {
        setRetryingFileIds((current) => current.filter((id) => id !== fileId));
      }
    }
  }, [beginTaskOperation, groupPath, invalidateLibrary, observeSourceAvailability, retryingFileIds, t, unavailableFileIds]);

  const releaseFileSource = useCallback((fileId: string, filename: string) => {
    setConfirmRequest({
      title: t("library.releaseSource"),
      message: t("library.releaseSourceConfirm", { name: filename }),
      acceptLabel: t("library.releaseSourceConfirmAction"),
      action: () => {
        setConfirmRequest(null);
        void (async () => {
          setReleasingFileIds((current) => [...current, fileId]);
          try {
            const fileDetail = await apiClient.releaseGroupLibraryFileSource(groupPath, fileId);
            observeSourceAvailability(fileId, fileDetail.source_available !== false);
            toast.success(t("library.releaseSourceSuccess"), {
              description: t("library.releaseSourceSuccessMessage"),
              duration: 2500,
            });
            await invalidateLibrary();
          } catch (error) {
            toast.error(t("common.error"), {
              description: error instanceof Error ? error.message : t("library.releaseSourceFailed"),
              duration: 5000,
            });
          } finally {
            setReleasingFileIds((current) => current.filter((id) => id !== fileId));
          }
        })();
      },
    });
  }, [groupPath, invalidateLibrary, observeSourceAvailability, t]);

  const retryAllFailedCount = failedCountQuery.data?.pagination.total ?? 0;
  const [retryAllBusy, setRetryAllBusy] = useState(false);

  const retryAllFailed = useCallback(() => {
    if (retryAllBusy || retryAllFailedCount === 0) {
      return;
    }
    setConfirmRequest({
      title: t("library.retryAllFailed"),
      message: t("library.retryAllConfirm", { count: retryAllFailedCount }),
      acceptLabel: t("library.retryAllConfirmAction"),
      action: () => {
        setConfirmRequest(null);
        void (async () => {
          setRetryAllBusy(true);
          const operation = beginTaskOperation();
          try {
            const collected: string[] = [];
            for (let currentPage = 1; ; currentPage += 1) {
              if (operation.isDisposed() || operation.signal.aborted) {
                return;
              }
              const response = await apiClient.getGroupLibraryResources(groupPath, {
                folderId,
                recursive: true,
                page: currentPage,
                pageSize: FAILED_COLLECT_PAGE_SIZE,
                query: "",
                status: "failed",
                sortBy: "updated_at",
                sortDirection: "desc",
              }, { signal: operation.signal });
              for (const item of response.items) {
                if (item.kind === "file") {
                  collected.push(item.id);
                }
              }
              if (
                collected.length >= response.pagination.total
                || response.items.length === 0
                || currentPage >= 1000
              ) {
                break;
              }
            }
            const unique = [...new Set(collected)];
            const candidates = unique.filter((id) => !unavailableFileIds.includes(id));
            let skipped = unique.length - candidates.length;
            const available: string[] = [];
            await Promise.all(candidates.map(async (fileId) => {
              try {
                const fileDetail = await apiClient.getGroupLibraryFile(groupPath, fileId, {
                  signal: operation.signal,
                });
                if (operation.isDisposed()) {
                  return;
                }
                observeSourceAvailability(fileId, fileDetail.source_available !== false);
                if (fileDetail.source_available === false) {
                  skipped += 1;
                } else {
                  available.push(fileId);
                }
              } catch {
                skipped += 1;
              }
            }));
            if (operation.isDisposed()) {
              return;
            }
            if (skipped > 0) {
              toast.warning(t("library.retryAllSkipped", { count: skipped }), { duration: 3500 });
            }
            if (available.length === 0) {
              await invalidateLibrary();
              return;
            }
            const task = await apiClient.submitTask({
              kind: "retry_file_batch",
              group_path: groupPath,
              items: available.map((file_id) => ({ file_id })),
            }, { signal: operation.signal });
            if (operation.isDisposed()) {
              return;
            }
            toast.success(t("library.retryAccepted"), {
              description: t("library.retryAcceptedMessage"),
              duration: 2500,
            });
            const results = await operation.settle([task]);
            if (operation.isDisposed()) {
              return;
            }
            await invalidateLibrary();
            if (operation.isDisposed()) {
              return;
            }
            if (results.some((result) => result.status === "failed")) {
              toast.error(t("common.error"), { description: t("library.retryFailed") });
            }
          } catch (error) {
            if (operation.isDisposed()) {
              return;
            }
            toast.error(t("common.error"), {
              description: error instanceof Error ? error.message : t("library.retryFailed"),
              duration: 5000,
            });
          } finally {
            operation.finish();
            if (!operation.isDisposed()) {
              setRetryAllBusy(false);
            }
          }
        })();
      },
    });
  }, [
    beginTaskOperation,
    folderId,
    groupPath,
    invalidateLibrary,
    observeSourceAvailability,
    retryAllBusy,
    retryAllFailedCount,
    t,
    unavailableFileIds,
  ]);

  const openSourceFolderCreate = useCallback(() => {
    setSourceFolder({
      open: true,
      busy: false,
      folderId: null,
      folderName: "",
      title: t("library.newSourceFolder"),
      value: defaultSourceTemplate(),
    });
  }, [t]);

  const openSourceFolderEditor = useCallback(async (entry: FileExplorerEntry) => {
    setSourceFolder((current) => ({ ...current, open: true, busy: true, title: t("library.editSourceConfig") }));
    try {
      const fileDetail = await apiClient.getGroupLibraryFile(groupPath, entry.id);
      const folderName = fileDetail.folder_path.split("/").filter(Boolean).at(-1) ?? "";
      setSourceFolder({
        open: true,
        busy: false,
        folderId: fileDetail.folder_id ?? null,
        folderName,
        title: t("library.editSourceConfig"),
        value: fileDetail.sections[0]?.preview_text || defaultSourceTemplate(),
      });
    } catch (error) {
      setSourceFolder((current) => ({ ...current, busy: false, open: false }));
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("library.detailLoadFailed"),
        duration: 5000,
      });
    }
  }, [groupPath, t]);

  const closeSourceFolder = useCallback(() => {
    setSourceFolder((current) => ({ ...current, open: false }));
  }, []);

  const saveSourceFolder = useCallback(async (payload: { folderName: string; value: string }) => {
    setSourceFolder((current) => ({ ...current, busy: true }));
    try {
      const parsed: unknown = JSON.parse(payload.value);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error(t("library.sourceConfigInvalid"));
      }
      const config = parsed as SourceConfigInput;
      if (sourceFolder.folderId) {
        await apiClient.updateGroupSourceFolderConfig(groupPath, sourceFolder.folderId, config);
      } else {
        await apiClient.createGroupSourceFolder(groupPath, {
          parent_folder_id: folderId,
          folder_name: payload.folderName.trim(),
          source_config: config,
        });
      }
      setSourceFolder((current) => ({ ...current, busy: false, open: false }));
      await invalidateLibrary();
      toast.success(sourceFolder.folderId ? t("common.save") : t("library.newSourceFolder"), {
        description: payload.folderName.trim() || config.source_key,
        duration: 2500,
      });
    } catch (error) {
      setSourceFolder((current) => ({ ...current, busy: false }));
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("common.save"),
        duration: 5000,
      });
    }
  }, [folderId, groupPath, invalidateLibrary, sourceFolder.folderId, t]);

  const syncSourceFolder = useCallback(async (sourceFolderId: string | null) => {
    if (!sourceFolderId) {
      return;
    }
    const operation = beginTaskOperation();
    try {
      const task = await apiClient.syncGroupSourceFolder(groupPath, sourceFolderId, {
        signal: operation.signal,
      });
      if (operation.isDisposed()) {
        return;
      }
      toast.success(t("sources.sync"), { description: t("sources.syncing"), duration: 2500 });
      const results = await operation.settle([task]);
      if (operation.isDisposed()) {
        return;
      }
      await invalidateLibrary();
      if (operation.isDisposed()) {
        return;
      }
      if (results.some((result) => result.status === "failed")) {
        toast.error(t("common.error"), { description: t("sources.syncFailed") });
      }
    } catch (error) {
      if (operation.isDisposed()) {
        return;
      }
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("sources.syncFailed"),
        duration: 5000,
      });
    } finally {
      operation.finish();
    }
  }, [beginTaskOperation, groupPath, invalidateLibrary, t]);

  const runScopedSearch = useCallback(async () => {
    const trimmed = scopedSearch.query.trim();
    if (!trimmed) {
      setScopedSearch((current) => ({ ...current, results: [], open: false }));
      return;
    }
    scopedSearchController.current?.abort();
    const controller = new AbortController();
    scopedSearchController.current = controller;
    setScopedSearch((current) => ({ ...current, loading: true }));
    const prefix = selectedFolder?.path?.replace(/\/+$/, "") || null;
    try {
      const response = await apiClient.search({
        query: trimmed,
        group_path: groupPath,
        limit: 20,
        page: 1,
      }, { signal: controller.signal });
      if (controller.signal.aborted) {
        return;
      }
      const results = response.items.filter((hit) => {
        if (!prefix) {
          return true;
        }
        if (!hit.is_library_file || !hit.library_path) {
          return false;
        }
        return hit.library_path === prefix || hit.library_path.startsWith(`${prefix}/`);
      });
      setScopedSearch((current) => ({
        ...current,
        loading: false,
        results,
        open: results.length > 0,
      }));
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        return;
      }
      setScopedSearch((current) => ({ ...current, loading: false, results: [] }));
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("search.scoped.loadFailed"),
        duration: 5000,
      });
    }
  }, [groupPath, scopedSearch.query, selectedFolder?.path, t]);

  const setScopedQuery = useCallback((value: string) => {
    setScopedSearch((current) => ({ ...current, query: value }));
  }, []);

  const setScopedOpen = useCallback((open: boolean) => {
    setScopedSearch((current) => ({ ...current, open }));
  }, []);

  const resourceContextEntry = resourceContextKey
    ? explorerEntries.find((entry) => entry.key === resourceContextKey) ?? null
    : null;

  const canRetryFile = useCallback((entry: ExplorerEntry) => (
    entry.kind === "file"
    && isRetryableFileStatus(entry.ingestStatus)
    && !unavailableFileIds.includes(entry.id)
  ), [unavailableFileIds]);

  const canRelease = useCallback((entry: ExplorerEntry) => (
    entry.kind === "file"
    && canReleaseFileSource(entry)
    && !unavailableFileIds.includes(entry.id)
  ), [unavailableFileIds]);

  return {
    actionBusy,
    canGoUp,
    canRelease,
    canRetryFile,
    changePage,
    changePageSize,
    changeQuery,
    changeSort,
    changeStatusFilter,
    clearSelection,
    closePreview,
    closeSourceFolder,
    confirmMove,
    confirmRequest,
    createBusy,
    createFolder,
    createFolderOpen,
    createTextFile,
    createTextOpen,
    deleteEntry,
    deleteSelected,
    detail,
    detailError: detailQuery.error,
    detailLoading: detailQuery.isLoading,
    entries,
    error: treeQuery.error ?? resourcesQuery.error,
    expandedKeys,
    explorerEntries,
    folderId,
    loading: treeQuery.isLoading || (resourcesQuery.isLoading && !resourcesQuery.data),
    moveDialog,
    moveOptions,
    navigateToFolder,
    openEntry,
    openSourceFolderCreate,
    openSourceFolderEditor,
    page,
    pageSize,
    parentFolderId: getParentFolderId(selectedFolderTrail),
    query,
    refresh,
    releaseFileSource,
    releaseSourceAfterUpload,
    releasingFileIds,
    resourceContextEntry,
    requestMoveEntry,
    resourcesPagination: resourcesQuery.data?.pagination ?? null,
    retryAllBusy,
    retryAllFailed,
    retryAllFailedCount,
    retryFile,
    retryingFileIds,
    runScopedSearch,
    saveSourceFolder,
    scopedSearch,
    statusFilter,
    selectedExplorerEntry,
    selectedFileEntry,
    selectedFileId,
    selectedFolder,
    selectedFolderSummary,
    selectedFolderTrail,
    selectedKeys,
    selectFile,
    setConfirmRequest,
    setCreateFolderOpen,
    setCreateTextOpen,
    setMoveDialogState: setMoveDialog,
    sortBy,
    sortDirection,
    updateSourceFolderName: (value: string) => {
      setSourceFolder((current) => ({ ...current, folderName: value }));
    },
    updateSourceFolderValue: (value: string) => {
      setSourceFolder((current) => ({ ...current, value }));
    },
    setReleaseSourceAfterUpload,
    setResourceContextKey,
    setScopedOpen,
    setScopedQuery,
    sourceFolder,
    syncSourceFolder,
    toggleFolderExpansion,
    toggleSelect,
    tree,
    treeError: treeQuery.error,
    treeLoading: treeQuery.isLoading,
    unavailableFileIds,
    uploadBusy,
    uploadFiles,
    groupName,
  };
}

export type LibraryWorkspaceState = ReturnType<typeof useLibraryWorkspace>;

export { findFolderById, folderKey };
