import { computed, ref, toValue, type MaybeRefOrGetter } from "vue";

import { apiClient, type LibraryFileSummary, type LibraryFolderNode, type LibraryTreeResponse } from "../../services/api";
import type { ExplorerEntry, FileExplorerEntry, FolderExplorerEntry, FolderSummary } from "../../types/library";
import { findFileLocation, findFolderById, findFolderTrail, flattenFolderOptions, folderKey } from "../../utils/library-tree";
import { useErrorToast } from "../use-error-toast";

interface UseProjectLibraryTreeOptions {
  groupPath: MaybeRefOrGetter<string>;
  statusLabel: (status: string) => string;
  t: (key: string, params?: Record<string, unknown>) => string;
}

export function useProjectLibraryTree({ groupPath, statusLabel, t }: UseProjectLibraryTreeOptions) {
  const showErrorToast = useErrorToast();
  const tree = ref<LibraryTreeResponse | null>(null);
  const treeError = ref<string | null>(null);
  const treeLoading = ref(false);
  const expandedTreeKeys = ref<Record<string, boolean>>({ [folderKey(null)]: true });
  const resourceSearchQuery = ref("");
  const selectedExplorerEntry = ref<ExplorerEntry | null>(null);
  const resourceContextEntry = ref<ExplorerEntry | null>(null);
  const selectedFolderId = ref<string | null>(null);
  const selectedFileId = ref<string | null>(null);
  let loadRequestId = 0;

  function updateExpandedForFolder(folderId: string | null) {
    if (!tree.value) return;
    const trail = findFolderTrail(tree.value.root, folderId);
    if (!trail) return;
    const nextExpandedKeys = { ...expandedTreeKeys.value };
    for (const folder of trail) {
      nextExpandedKeys[folderKey(folder.folder_id ?? null)] = true;
    }
    expandedTreeKeys.value = nextExpandedKeys;
  }

  async function replaceSelection(folderId: string | null, fileId: string | null) {
    selectedFolderId.value = folderId;
    selectedFileId.value = fileId;
  }

  const selectedFolder = computed(() => {
    if (!tree.value) return null;
    return findFolderById(tree.value.root, selectedFolderId.value) ?? tree.value.root;
  });

  const selectedFolderTrail = computed(() => {
    if (!tree.value) return [];
    return findFolderTrail(tree.value.root, selectedFolderId.value) ?? [tree.value.root];
  });

  const moveOptions = computed(() => {
    if (!tree.value) return [] as Array<{ label: string; value: string | null }>;
    return flattenFolderOptions(tree.value.root);
  });

  function isSourceFolderNode(folder: LibraryFolderNode): boolean {
    return folder.files.some((file) => file.filename.toLowerCase() === "source.json");
  }

  function isSourceRecordsFolderNode(folder: LibraryFolderNode): boolean {
    return folder.name === "records" && !!folder.parent_folder_id;
  }

  function buildFolderExplorerEntry(folder: LibraryFolderNode, depth: number, parentFolderId: string | null): FolderExplorerEntry {
    return {
      key: `folder:${folderKey(folder.folder_id ?? null)}`,
      kind: "folder",
      id: folder.folder_id ?? null,
      depth,
      name: folder.name,
      parentFolderId,
      path: folder.path,
      updatedAt: null,
      childFolderCount: folder.children.length,
      fileCount: folder.files.length,
      isSourceFolder: isSourceFolderNode(folder),
      isSourceRecordsFolder: isSourceRecordsFolderNode(folder),
      processingCount: folder.processing_count,
      folder,
    };
  }

  function buildFileExplorerEntry(
    file: LibraryFileSummary,
    depth: number,
    parentFolderId: string | null,
    parentPath: string,
    parentFolder: LibraryFolderNode,
  ): FileExplorerEntry {
    return {
      key: `file:${file.file_id}`,
      kind: "file",
      id: file.file_id,
      depth,
      name: file.filename,
      parentFolderId,
      path: parentPath,
      updatedAt: file.updated_at,
      mediaType: file.media_type,
      sizeBytes: file.size_bytes,
      ingestStatus: file.ingest_status,
      errorMessage: file.error_message ?? null,
      isSourceConfigFile: file.filename.toLowerCase() === "source.json" && isSourceFolderNode(parentFolder),
      isSourceRecordFile: parentFolder.name === "records",
      file,
    };
  }

  const explorerEntries = computed<ExplorerEntry[]>(() => {
    if (!tree.value) return [];
    const rows: ExplorerEntry[] = [];
    function appendFolderRows(folder: LibraryFolderNode, depth: number) {
      const sortedChildren = [...folder.children].sort((left, right) => left.name.localeCompare(right.name, undefined, { sensitivity: "base" }));
      const sortedFiles = [...folder.files].sort((left, right) => left.filename.localeCompare(right.filename, undefined, { sensitivity: "base" }));
      for (const childFolder of sortedChildren) {
        rows.push(buildFolderExplorerEntry(childFolder, depth, folder.folder_id ?? null));
        if (expandedTreeKeys.value[folderKey(childFolder.folder_id ?? null)]) {
          appendFolderRows(childFolder, depth + 1);
        }
      }
      for (const file of sortedFiles) {
        rows.push(buildFileExplorerEntry(file, depth, folder.folder_id ?? null, folder.path, folder));
      }
    }
    appendFolderRows(tree.value.root, 0);
    return rows;
  });

  const filteredExplorerEntries = computed(() => {
    const query = resourceSearchQuery.value.trim().toLowerCase();
    if (!query) return explorerEntries.value;
    return explorerEntries.value.filter((entry) => {
      const values = entry.kind === "folder"
        ? [entry.name, entry.path, t("library.folderType")]
        : [entry.name, entry.path, entry.mediaType, entry.errorMessage ?? "", statusLabel(entry.ingestStatus)];
      return values.some((value) => value.toLowerCase().includes(query));
    });
  });

  const filteredResourceCountLabel = computed(() => t("library.resourceCount", { count: filteredExplorerEntries.value.length }));

  const selectedFolderSummary = computed<FolderSummary | null>(() => {
    if (!selectedFolder.value) return null;
    return {
      name: selectedFolder.value.name,
      path: selectedFolder.value.path,
      childFolderCount: selectedFolder.value.children.length,
      fileCount: selectedFolder.value.files.length,
      isSourceFolder: isSourceFolderNode(selectedFolder.value),
      isSourceRecordsFolder: isSourceRecordsFolderNode(selectedFolder.value),
      processingCount: selectedFolder.value.processing_count,
    };
  });

  const breadcrumbHome = computed(() => ({ label: t("library.rootFolder"), onSelect: () => { void selectFolder(null); } }));
  const breadcrumbItems = computed(() => selectedFolderTrail.value.slice(1).map((folder) => ({ label: folder.name, onSelect: () => { void selectFolder(folder.folder_id ?? null); } })));

  async function loadTree() {
    const requestId = ++loadRequestId;
    treeLoading.value = !tree.value;
    treeError.value = null;
    try {
      const nextTree = await apiClient.getGroupLibraryTree(toValue(groupPath));
      if (requestId !== loadRequestId) return;
      tree.value = nextTree;
      if (selectedFolderId.value && !findFolderById(nextTree.root, selectedFolderId.value)) {
        await replaceSelection(null, selectedFileId.value);
        return;
      }
      if (selectedFileId.value) {
        const location = findFileLocation(nextTree.root, selectedFileId.value);
        if (!location) {
          await replaceSelection(selectedFolderId.value, null);
          return;
        }
        updateExpandedForFolder(location.folder.folder_id ?? null);
        if (selectedFolderId.value !== location.folder.folder_id) {
          await replaceSelection(location.folder.folder_id ?? null, selectedFileId.value);
          return;
        }
      } else {
        updateExpandedForFolder(selectedFolderId.value);
      }
    } catch (error) {
      if (requestId !== loadRequestId) return;
      treeError.value = t("library.loadFailed");
      showErrorToast(error, t("library.loadFailed"));
    } finally {
      if (requestId === loadRequestId) {
        treeLoading.value = false;
      }
    }
  }

  function resetTree() {
    loadRequestId += 1;
    tree.value = null;
    treeError.value = null;
    selectedExplorerEntry.value = null;
    selectedFolderId.value = null;
    selectedFileId.value = null;
    expandedTreeKeys.value = { [folderKey(null)]: true };
  }

  async function selectFolder(folderId: string | null) {
    updateExpandedForFolder(folderId);
    await replaceSelection(folderId, null);
  }

  async function selectFile(fileId: string) {
    await replaceSelection(selectedFolder.value?.folder_id ?? null, fileId);
  }

  function toggleFolderExpansion(folderId: string | null) {
    const key = folderKey(folderId);
    if (expandedTreeKeys.value[key]) {
      const nextExpandedKeys = { ...expandedTreeKeys.value };
      delete nextExpandedKeys[key];
      expandedTreeKeys.value = nextExpandedKeys;
    } else {
      expandedTreeKeys.value = { ...expandedTreeKeys.value, [key]: true };
    }
  }

  function syncSelectedExplorerEntry(entries: ExplorerEntry[]) {
    if (selectedFileId.value) {
      selectedExplorerEntry.value = entries.find((entry) => entry.kind === "file" && entry.id === selectedFileId.value) ?? null;
      return;
    }
    selectedExplorerEntry.value = entries.find((entry) => entry.kind === "folder" && entry.id === selectedFolderId.value) ?? null;
  }

  return {
    breadcrumbHome,
    breadcrumbItems,
    expandedTreeKeys,
    explorerEntries,
    filteredExplorerEntries,
    filteredResourceCountLabel,
    loadTree,
    moveOptions,
    replaceSelection,
    resetTree,
    resourceContextEntry,
    resourceSearchQuery,
    selectFile,
    selectedExplorerEntry,
    selectedFileId,
    selectedFolder,
    selectedFolderId,
    selectedFolderSummary,
    selectedFolderTrail,
    selectFolder,
    syncSelectedExplorerEntry,
    toggleFolderExpansion,
    tree,
    treeError,
    treeLoading,
    updateExpandedForFolder,
  };
}
