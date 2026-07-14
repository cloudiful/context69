import { computed, ref } from "vue";
import type { Router, RouteLocationNormalizedLoadedGeneric } from "vue-router";

import { apiClient, type LibraryFolderNode, type LibraryTreeResponse } from "../../services/api";
import type { ExplorerEntry, FolderSummary } from "../../types/library";
import { useErrorToast } from "../use-error-toast";
import {
  findFolderById,
  findFolderTrail,
  flattenFolderOptions,
  folderKey,
  queryValue,
} from "../../utils/library-tree";

interface UseLibraryTreeOptions {
  route: RouteLocationNormalizedLoadedGeneric;
  router: Router;
  t: (key: string, params?: Record<string, unknown>) => string;
}

export function useLibraryTree({ route, router, t }: UseLibraryTreeOptions) {
  const showErrorToast = useErrorToast();
  const tree = ref<LibraryTreeResponse | null>(null);
  const treeLoading = ref(false);
  const expandedTreeKeys = ref<Record<string, boolean>>({
    [folderKey(null)]: true,
  });
  const selectedExplorerEntry = ref<ExplorerEntry | null>(null);
  const resourceContextEntry = ref<ExplorerEntry | null>(null);

  const selectedFolderId = computed(() => queryValue(route.query.folder));
  const selectedFileId = computed(() => queryValue(route.query.file));

  function updateExpandedForFolder(folderId: string | null) {
    if (!tree.value) {
      return;
    }

    const trail = findFolderTrail(tree.value.root, folderId);
    if (!trail) {
      return;
    }

    const nextExpandedKeys = { ...expandedTreeKeys.value };
    for (const folder of trail) {
      nextExpandedKeys[folderKey(folder.folder_id ?? null)] = true;
    }
    expandedTreeKeys.value = nextExpandedKeys;
  }

  async function replaceQuery(folderId: string | null, fileId: string | null) {
    const query = { ...route.query } as Record<string, string>;

    if (folderId) {
      query.folder = folderId;
    } else {
      delete query.folder;
    }

    if (fileId) {
      query.file = fileId;
    } else {
      delete query.file;
    }

    await router.replace({
      name: "library",
      query,
    });
  }

  const selectedFolder = computed(() => {
    if (!tree.value) {
      return null;
    }
    return findFolderById(tree.value.root, selectedFolderId.value) ?? tree.value.root;
  });

  const selectedFolderTrail = computed(() => {
    if (!tree.value) {
      return [];
    }
    return findFolderTrail(tree.value.root, selectedFolderId.value) ?? [tree.value.root];
  });

  const moveOptions = computed(() => {
    if (!tree.value) {
      return [] as Array<{ label: string; value: string }>;
    }

    return flattenFolderOptions(tree.value.root);
  });

  function isSourceFolderNode(folder: LibraryFolderNode): boolean {
    return folder.files.some((file) => file.filename.toLowerCase() === "source.json");
  }

  function isSourceRecordsFolderNode(folder: LibraryFolderNode): boolean {
    return folder.name === "records" && !!folder.parent_folder_id;
  }

  const selectedFolderSummary = computed<FolderSummary | null>(() => {
    if (!selectedFolder.value) {
      return null;
    }

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

  const breadcrumbHome = computed(() => ({
    label: t("library.rootFolder"),
    onSelect: () => {
      void selectFolder(null);
    },
  }));

  const breadcrumbItems = computed(() => selectedFolderTrail.value
    .slice(1)
    .map((folder) => ({
      label: folder.name,
      onSelect: () => {
        void selectFolder(folder.folder_id ?? null);
      },
    })));

  async function loadTree() {
    treeLoading.value = !tree.value;

    try {
      const nextTree = await apiClient.getLibraryTree();
      tree.value = nextTree;

      if (selectedFolderId.value && !findFolderById(nextTree.root, selectedFolderId.value)) {
        await replaceQuery(null, selectedFileId.value);
        return;
      }

      updateExpandedForFolder(selectedFolderId.value);
    } catch (error) {
      showErrorToast(error, t("library.loadFailed"));
    } finally {
      treeLoading.value = false;
    }
  }

  async function selectFolder(folderId: string | null) {
    updateExpandedForFolder(folderId);
    await replaceQuery(folderId, null);
  }

  async function selectFile(fileId: string) {
    await replaceQuery(selectedFolder.value?.folder_id ?? null, fileId);
  }

  function toggleFolderExpansion(folderId: string | null) {
    const key = folderKey(folderId);

    if (expandedTreeKeys.value[key]) {
      const nextExpandedKeys = { ...expandedTreeKeys.value };
      delete nextExpandedKeys[key];
      expandedTreeKeys.value = nextExpandedKeys;
    } else {
      expandedTreeKeys.value = {
        ...expandedTreeKeys.value,
        [key]: true,
      };
    }
  }

  async function refreshLibrary(loadDetail: (fileId: string | null) => Promise<void>) {
    await loadTree();
    await loadDetail(selectedFileId.value);
  }

  function syncSelectedExplorerEntry(entries: ExplorerEntry[]) {
    if (selectedFileId.value) {
      selectedExplorerEntry.value = entries.find(
        (entry) => entry.kind === "file" && entry.id === selectedFileId.value,
      ) ?? null;
      return;
    }

    selectedExplorerEntry.value = entries.find(
      (entry) => entry.kind === "folder" && entry.id === selectedFolderId.value,
    ) ?? null;
  }

  return {
    breadcrumbHome,
    breadcrumbItems,
    expandedTreeKeys,
    loadTree,
    moveOptions,
    refreshLibrary,
    replaceQuery,
    resourceContextEntry,
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
    treeLoading,
    updateExpandedForFolder,
  };
}
