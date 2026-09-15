import type { LibraryIngestStatus } from "../../services/api";
import type { ExplorerEntry } from "../../types/library";

// A file is offered for reprocessing while it is not a confirmed success.
// File states are terminal-only (`succeeded`/`failed`, issue #400);
// processing is derived from active tasks, so only `failed` is retryable.
export const RETRYABLE_FILE_STATUSES: readonly LibraryIngestStatus[] = [
  "failed",
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
