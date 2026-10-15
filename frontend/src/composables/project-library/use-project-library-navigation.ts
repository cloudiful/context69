import type { LocationQuery } from "vue-router";

import type { LibraryFolderNode } from "../../services/api";
import { findFolderById, queryValue } from "../../utils/library-tree";

export const LIBRARY_FOLDER_QUERY_KEY = "folder";
const LEGACY_ROOT_TOKENS = new Set(["null", "__root__"]);

export function normalizeLibraryFolderQuery(value: unknown): string | null {
  const parsed = queryValue(value);
  if (!parsed) return null;
  const trimmed = parsed.trim();
  if (!trimmed) return null;
  if (LEGACY_ROOT_TOKENS.has(trimmed)) return null;
  return trimmed;
}

export function readLibraryFolderQuery(query: LocationQuery): string | null {
  return normalizeLibraryFolderQuery(query[LIBRARY_FOLDER_QUERY_KEY]);
}

export function buildLibraryFolderQuery(
  current: LocationQuery,
  folderId: string | null,
  options: { clearFile?: boolean } = {},
): LocationQuery {
  const next: LocationQuery = { ...current };
  if (folderId === null) {
    delete next[LIBRARY_FOLDER_QUERY_KEY];
  } else {
    next[LIBRARY_FOLDER_QUERY_KEY] = folderId;
  }
  if (options.clearFile !== false) {
    delete next.file;
  }
  return next;
}

export function resolveLibraryFolderSelection(
  root: LibraryFolderNode | null | undefined,
  folderId: string | null,
): string | null {
  if (!root) return null;
  if (folderId === null) return null;
  return findFolderById(root, folderId) ? folderId : null;
}

export function isValidLibraryFolder(root: LibraryFolderNode | null | undefined, folderId: string | null): boolean {
  if (folderId === null) return true;
  if (!root) return false;
  return findFolderById(root, folderId) !== null;
}

export function getParentFolderId(
  trail: LibraryFolderNode[],
): string | null {
  if (trail.length < 2) return null;
  const parent = trail[trail.length - 2];
  return parent?.folder_id ?? null;
}
