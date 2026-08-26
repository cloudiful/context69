import type { LibraryIngestStatus } from "../services/api";
import { useI18n } from "vue-i18n";

type Translate = (key: string, params?: Record<string, unknown>) => string;

export function libraryStatusSeverity(status: LibraryIngestStatus): "success" | "error" | "warning" | "neutral" {
  switch (status) {
    case "succeeded":
      return "success";
    case "failed":
      return "error";
    case "running":
      return "warning";
    default:
      return "neutral";
  }
}

export function libraryStatusLabel(t: Translate, status: LibraryIngestStatus): string {
  return t(`library.status.${status}`);
}

export function createLibraryStatusHelpers() {
  const { t } = useI18n();

  return {
    statusLabel(status: LibraryIngestStatus): string {
      return libraryStatusLabel(t, status);
    },
    statusSeverity: libraryStatusSeverity,
  };
}

// Canonical library dependency keys emitted by the processing queue.
// Order is stable for filter UIs and tests; new keys must be appended.
export const LIBRARY_DEPENDENCY_KEYS = ["s3", "docling", "embedding", "qdrant"] as const;
export type LibraryDependencyKey = typeof LIBRARY_DEPENDENCY_KEYS[number];

// Legacy internal keys produced by older task records are normalized here so
// users always see the canonical label. New tasks should write canonical keys
// directly; do not add aliases for keys that never shipped.
const LEGACY_LIBRARY_DEPENDENCY_ALIASES: Readonly<Record<string, LibraryDependencyKey>> = {
  embedding_vector: "embedding",
};

export function canonicalLibraryDependencyKey(key: string | null): LibraryDependencyKey | null {
  if (!key) return null;
  const alias = LEGACY_LIBRARY_DEPENDENCY_ALIASES[key];
  if (alias) return alias;
  return (LIBRARY_DEPENDENCY_KEYS as readonly string[]).includes(key)
    ? (key as LibraryDependencyKey)
    : null;
}

// Resolves a dependency key to its localized label. Unknown keys (including
// future additions) are returned as-is so they remain visible instead of being
// hidden by an unrecognized filter.
export function libraryDependencyLabel(t: Translate, dependencyKey: string | null): string {
  if (!dependencyKey) return "--";
  const canonical = canonicalLibraryDependencyKey(dependencyKey);
  if (canonical) return t(`processingQueue.dependencies.${canonical}`);
  return dependencyKey;
}