import type { LibraryFileSummary, LibraryFolderNode } from "../services/api";

export interface FileLocation {
  folder: LibraryFolderNode;
  file: LibraryFileSummary;
}

export function folderKey(folderId: string | null): string {
  return folderId ?? "__root__";
}

export function queryValue(value: unknown): string | null {
  if (typeof value === "string" && value) {
    return value;
  }
  if (Array.isArray(value) && typeof value[0] === "string" && value[0]) {
    return value[0];
  }
  return null;
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
) {
  if (node.folder_id) {
    rows.push({
      value: node.folder_id,
      label: node.path,
    });
  }

  for (const child of node.children) {
    flattenFolderOptions(child, rows);
  }

  return rows;
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
