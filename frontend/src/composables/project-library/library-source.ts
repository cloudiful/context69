import type { LibraryIngestStatus } from "../../services/api";
import type { ExplorerEntry } from "../../types/library";

// A file is offered for reprocessing while it is not a confirmed success.
// `pending`/`running` are stale projections once no active task exists, so the
// browser exposes retry for them as well; the backend rejects a live task.
export const RETRYABLE_FILE_STATUSES: readonly LibraryIngestStatus[] = [
  "pending",
  "running",
  "failed",
  "cancelled",
];

export function isRetryableFileStatus(status: LibraryIngestStatus): boolean {
  return RETRYABLE_FILE_STATUSES.includes(status);
}

// Releasing is only meaningful for a completed, user-owned file. Sync control
// files are never releasable.
export function canReleaseFileSource(entry: ExplorerEntry): boolean {
  return entry.kind === "file"
    && entry.ingestStatus === "succeeded"
    && !entry.isSourceConfigFile
    && !entry.isSourceRecordFile;
}
