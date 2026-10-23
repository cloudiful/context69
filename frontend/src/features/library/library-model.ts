import type {
  GroupResponse,
  LibraryFileSummary,
  LibraryFolderNode,
  LibraryIngestStatus,
  LibraryResourceItem,
} from "@/services/api";

export interface ExplorerBaseEntry {
  key: string;
  name: string;
  depth: number;
  parentFolderId: string | null;
  path: string;
  updatedAt: string | null;
}

export interface FolderExplorerEntry extends ExplorerBaseEntry {
  kind: "folder";
  id: string | null;
  childFolderCount: number;
  fileCount: number;
  isSourceFolder: boolean;
  isSourceRecordsFolder: boolean;
  processingCount: number;
  folder: LibraryFolderNode;
}

export interface FileExplorerEntry extends ExplorerBaseEntry {
  kind: "file";
  id: string;
  mediaType: string;
  sizeBytes: number;
  ingestStatus: LibraryIngestStatus;
  errorMessage: string | null;
  isSourceConfigFile: boolean;
  isSourceRecordFile: boolean;
  file: LibraryFileSummary;
}

export type ExplorerEntry = FolderExplorerEntry | FileExplorerEntry;

export interface GroupExplorerEntry extends ExplorerBaseEntry {
  kind: "group";
  id: number;
  group: GroupResponse;
  visibility: GroupResponse["visibility"];
}

export type LibraryBrowserEntry = ExplorerEntry | GroupExplorerEntry;

export interface FolderSummary {
  childFolderCount: number;
  fileCount: number;
  isSourceFolder: boolean;
  isSourceRecordsFolder: boolean;
  name: string;
  path: string;
  processingCount: number;
}

export function folderKey(folderId: string | null): string {
  return folderId ?? "__root__";
}

export function findFolderById(
  node: LibraryFolderNode,
  folderId: string | null,
): LibraryFolderNode | null {
  if (!folderId) {
    return node;
  }
  if (node.folder_id === folderId) {
    return node;
  }
  for (const child of node.children) {
    const match = findFolderById(child, folderId);
    if (match) {
      return match;
    }
  }
  return null;
}

export function findFolderTrail(
  node: LibraryFolderNode,
  folderId: string | null,
  trail: LibraryFolderNode[] = [],
): LibraryFolderNode[] | null {
  const nextTrail = [...trail, node];
  if (folderId === null && node.folder_id === null) {
    return nextTrail;
  }
  if (node.folder_id === folderId) {
    return nextTrail;
  }
  for (const child of node.children) {
    const match = findFolderTrail(child, folderId, nextTrail);
    if (match) {
      return match;
    }
  }
  return null;
}

export function collectDescendantFolderIds(
  node: LibraryFolderNode,
  result: string[] = [],
): string[] {
  if (node.folder_id) {
    result.push(node.folder_id);
  }
  for (const child of node.children) {
    collectDescendantFolderIds(child, result);
  }
  return result;
}

export function flattenFolderOptions(
  node: LibraryFolderNode,
  rows: Array<{ value: string; label: string }> = [],
): Array<{ value: string; label: string }> {
  if (node.folder_id) {
    rows.push({ value: node.folder_id, label: node.path });
  }
  for (const child of node.children) {
    flattenFolderOptions(child, rows);
  }
  return rows;
}

export interface FileLocation {
  folder: LibraryFolderNode;
  file: LibraryFileSummary;
}

export function findFileLocation(
  node: LibraryFolderNode,
  fileId: string,
): FileLocation | null {
  const file = node.files.find((entry) => entry.file_id === fileId);
  if (file) {
    return { folder: node, file };
  }
  for (const child of node.children) {
    const match = findFileLocation(child, fileId);
    if (match) {
      return match;
    }
  }
  return null;
}

export const LIBRARY_FOLDER_QUERY_KEY = "folder";
export const LIBRARY_FILE_QUERY_KEY = "file";
const LEGACY_ROOT_TOKENS = new Set(["null", "__root__"]);

export function normalizeLibraryFolderQuery(value: string | null | undefined): string | null {
  if (!value) {
    return null;
  }
  const trimmed = value.trim();
  if (!trimmed || LEGACY_ROOT_TOKENS.has(trimmed)) {
    return null;
  }
  return trimmed;
}

export function readLibraryFolderQuery(params: URLSearchParams): string | null {
  return normalizeLibraryFolderQuery(params.get(LIBRARY_FOLDER_QUERY_KEY));
}

export function readLibraryFileQuery(params: URLSearchParams): string | null {
  const value = params.get(LIBRARY_FILE_QUERY_KEY);
  return value ? value : null;
}

export function buildLibraryFolderQuery(
  current: URLSearchParams,
  folderId: string | null,
  options: { clearFile?: boolean } = {},
): URLSearchParams {
  const next = new URLSearchParams(current);
  if (folderId === null) {
    next.delete(LIBRARY_FOLDER_QUERY_KEY);
  } else {
    next.set(LIBRARY_FOLDER_QUERY_KEY, folderId);
  }
  if (options.clearFile !== false) {
    next.delete(LIBRARY_FILE_QUERY_KEY);
  }
  return next;
}

export function isValidLibraryFolder(
  root: LibraryFolderNode | null | undefined,
  folderId: string | null,
): boolean {
  if (folderId === null) {
    return true;
  }
  if (!root) {
    return false;
  }
  return findFolderById(root, folderId) !== null;
}

export function getParentFolderId(trail: LibraryFolderNode[]): string | null {
  if (trail.length < 2) {
    return null;
  }
  return trail[trail.length - 2]?.folder_id ?? null;
}

export function libraryStatusLabel(
  t: (key: string) => string,
  status: LibraryIngestStatus,
): string {
  return t(`library.status.${status}`);
}

export function isRetryableFileStatus(status: LibraryIngestStatus): boolean {
  return status === "failed";
}

/** Releasing is only meaningful for a completed, user-owned file. */
export function canReleaseFileSource(entry: ExplorerEntry): boolean {
  return entry.kind === "file"
    && entry.ingestStatus === "succeeded"
    && !entry.isSourceConfigFile
    && !entry.isSourceRecordFile;
}

export function canMutateEntry(entry: LibraryBrowserEntry): boolean {
  if (entry.kind === "group") {
    return true;
  }
  if (entry.kind === "folder") {
    return !entry.isSourceRecordsFolder;
  }
  return !entry.isSourceConfigFile && !entry.isSourceRecordFile;
}

function isSourceFolderNode(folder: LibraryFolderNode): boolean {
  return folder.files.some((file) => file.filename.toLowerCase() === "source.json");
}

function isSourceRecordsFolderNode(folder: LibraryFolderNode): boolean {
  return folder.name === "records" && !!folder.parent_folder_id;
}

export function buildFolderExplorerEntry(
  folder: LibraryFolderNode,
  depth: number,
  parentFolderId: string | null,
): FolderExplorerEntry {
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

export function buildFileExplorerEntry(
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

/** Expands the tree into the flat row list used by the library table. */
export function buildExplorerEntries(
  tree: LibraryFolderNode,
  expandedKeys: Record<string, boolean>,
): ExplorerEntry[] {
  const rows: ExplorerEntry[] = [];
  const appendFolderRows = (folder: LibraryFolderNode, depth: number) => {
    const children = [...folder.children].sort((left, right) => (
      left.name.localeCompare(right.name, undefined, { sensitivity: "base" })
    ));
    const files = [...folder.files].sort((left, right) => (
      left.filename.localeCompare(right.filename, undefined, { sensitivity: "base" })
    ));
    for (const child of children) {
      rows.push(buildFolderExplorerEntry(child, depth, folder.folder_id ?? null));
      if (expandedKeys[folderKey(child.folder_id ?? null)]) {
        appendFolderRows(child, depth + 1);
      }
    }
    for (const file of files) {
      rows.push(buildFileExplorerEntry(file, depth, folder.folder_id ?? null, folder.path, folder));
    }
  };
  appendFolderRows(tree, 0);
  return rows;
}

export function folderResourceEntry(
  item: LibraryResourceItem,
  parent: LibraryFolderNode,
): ExplorerEntry {
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

export function fileResourceEntry(
  item: LibraryResourceItem,
  parent: LibraryFolderNode,
): ExplorerEntry {
  const file: LibraryFileSummary = {
    file_id: item.id,
    group_key: item.group_key,
    group_path: item.group_path,
    visibility: item.visibility,
    folder_id: item.parent_folder_id ?? null,
    filename: item.name,
    media_type: item.media_type ?? "application/octet-stream",
    size_bytes: item.size_bytes ?? 0,
    ingest_status: item.ingest_status ?? "failed",
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

export function buildGroupExplorerEntry(group: GroupResponse): GroupExplorerEntry {
  return {
    key: `group:${group.group_id}`,
    kind: "group",
    id: group.group_id,
    depth: 0,
    name: group.name,
    parentFolderId: null,
    path: group.group_path ?? group.group_key,
    updatedAt: group.updated_at,
    visibility: group.visibility,
    group,
  };
}
