import { computed, ref, toValue, type MaybeRefOrGetter } from "vue";

import {
  apiClient,
  type LibraryFileSummary,
  type LibraryFolderNode,
  type LibraryResourceItem,
  type LibraryResourceSortBy,
  type SortDirection,
} from "../../services/api";
import type { ExplorerEntry } from "../../types/library";
import { useErrorToast } from "../use-error-toast";

interface Options {
  groupPath: MaybeRefOrGetter<string>;
  folder: MaybeRefOrGetter<LibraryFolderNode | null>;
  t: (key: string) => string;
}

export function useProjectLibraryPage({ groupPath, folder, t }: Options) {
  const showErrorToast = useErrorToast();
  const entries = ref<ExplorerEntry[]>([]);
  const error = ref<string | null>(null);
  const loading = ref(false);
  const page = ref(1);
  const pageSize = ref(50);
  const total = ref(0);
  const sortBy = ref<LibraryResourceSortBy>("updated_at");
  const sortDirection = ref<SortDirection>("desc");
  const query = ref("");
  let requestId = 0;

  const first = computed(() => (page.value - 1) * pageSize.value);
  const sortOrder = computed(() => sortDirection.value === "asc" ? 1 : -1);

  function folderEntry(item: LibraryResourceItem, parent: LibraryFolderNode): ExplorerEntry {
    const path = `${parent.path.replace(/\/$/, "")}/${item.name}`;
    const node: LibraryFolderNode = {
      group_key: item.group_key,
      group_path: item.group_path,
      visibility: item.visibility,
      folder_id: item.id,
      parent_folder_id: item.parent_folder_id ?? null,
      name: item.name,
      path,
      processing_count: item.processing_count,
      children: [],
      files: [],
    };
    return {
      key: `folder:${item.id}`,
      kind: "folder",
      id: item.id,
      depth: 0,
      name: item.name,
      parentFolderId: item.parent_folder_id ?? null,
      path,
      updatedAt: item.updated_at,
      childFolderCount: item.child_folder_count,
      fileCount: item.file_count,
      isSourceFolder: item.is_source_folder,
      isSourceRecordsFolder: item.is_source_records_folder,
      processingCount: item.processing_count,
      folder: node,
    };
  }

  function fileEntry(item: LibraryResourceItem, parent: LibraryFolderNode): ExplorerEntry {
    const file: LibraryFileSummary = {
      file_id: item.id,
      group_key: item.group_key,
      group_path: item.group_path,
      visibility: item.visibility,
      folder_id: item.parent_folder_id ?? null,
      filename: item.name,
      media_type: item.media_type ?? "application/octet-stream",
      size_bytes: item.size_bytes ?? 0,
      ingest_status: item.ingest_status ?? "pending",
      error_message: item.error_message ?? null,
      created_at: item.created_at,
      updated_at: item.updated_at,
      ingested_at: null,
    };
    return {
      key: `file:${item.id}`,
      kind: "file",
      id: item.id,
      depth: 0,
      name: item.name,
      parentFolderId: item.parent_folder_id ?? null,
      path: parent.path,
      updatedAt: item.updated_at,
      mediaType: file.media_type,
      sizeBytes: file.size_bytes,
      ingestStatus: file.ingest_status,
      errorMessage: file.error_message ?? null,
      isSourceConfigFile: item.name.toLowerCase() === "source.json",
      isSourceRecordFile: parent.name === "records" && !!parent.parent_folder_id,
      file,
    };
  }

  async function loadPage() {
    const parent = toValue(folder);
    if (!parent) return;
    const currentRequest = ++requestId;
    loading.value = true;
    error.value = null;
    try {
      const response = await apiClient.getGroupLibraryResources(toValue(groupPath), {
        folderId: parent.folder_id ?? null,
        page: page.value,
        pageSize: pageSize.value,
        query: query.value.trim(),
        sortBy: sortBy.value,
        sortDirection: sortDirection.value,
      });
      if (currentRequest !== requestId) return;
      entries.value = response.items.map((item) => item.kind === "folder"
        ? folderEntry(item, parent)
        : fileEntry(item, parent));
      total.value = response.total;
    } catch (cause) {
      if (currentRequest !== requestId) return;
      error.value = t("library.loadFailed");
      showErrorToast(cause, error.value);
    } finally {
      if (currentRequest === requestId) loading.value = false;
    }
  }

  async function changePage(nextFirst: number, nextRows: number) {
    pageSize.value = nextRows;
    page.value = Math.floor(nextFirst / nextRows) + 1;
    await loadPage();
  }

  async function changeSort(field: LibraryResourceSortBy, order: number) {
    sortBy.value = field;
    sortDirection.value = order === 1 ? "asc" : "desc";
    page.value = 1;
    await loadPage();
  }

  function reset() {
    requestId += 1;
    entries.value = [];
    error.value = null;
    page.value = 1;
    total.value = 0;
  }

  return {
    changePage,
    changeSort,
    entries,
    error,
    first,
    loadPage,
    loading,
    pageSize,
    page,
    query,
    reset,
    sortBy,
    sortOrder,
    total,
  };
}
