import {
  DOCUMENT_PAGE_SIZE,
  documentBlocks,
  documentPageCount,
  type DocumentBlock,
} from "@/features/documents/document-model";
import type { DocumentResponse, LibraryDocumentSectionPreview } from "@/services/api";

/** One associated document of a library file, in the file's declared order. */
export interface LibraryPreviewDocumentRef {
  documentId: number;
  sectionKey: string;
  title: string;
  sortOrder: number;
}

/**
 * The documents a library file is made of, in the declared section order.
 * A document that several sections point at is read once: the repeated section
 * ids must not turn into repeated requests.
 */
export function libraryPreviewDocumentRefs(
  sections: LibraryDocumentSectionPreview[],
): LibraryPreviewDocumentRef[] {
  const ordered = [...sections].sort((left, right) => (
    left.sort_order - right.sort_order || left.section_key.localeCompare(right.section_key)
  ));
  const seen = new Set<number>();
  const refs: LibraryPreviewDocumentRef[] = [];
  for (const section of ordered) {
    if (seen.has(section.document_id)) {
      continue;
    }
    seen.add(section.document_id);
    refs.push({
      documentId: section.document_id,
      sectionKey: section.section_key,
      title: section.title,
      sortOrder: section.sort_order,
    });
  }
  return refs;
}

/** One stored block of one associated document, ready for the reader page. */
export interface LibraryPreviewBlock {
  block: DocumentBlock;
  documentId: number;
  chunkIndex: number;
  /** The section title, carried by the first block of its document only. */
  sectionTitle: string | null;
}

/**
 * The read state of one associated document.
 *
 * `ready` is the only state whose content is current. `refreshing` keeps the
 * cached copy of a background refresh, `failed` has no current content at all,
 * and `pending` was never read.
 */
export type LibraryPreviewDocumentStatus = "pending" | "ready" | "refreshing" | "failed";

export interface LibraryPreviewDocumentState {
  documentId: number;
  title: string;
  status: LibraryPreviewDocumentStatus;
  document?: DocumentResponse;
  error?: unknown;
}

/** The query state of one associated document, as the native query reports it. */
export interface LibraryPreviewDocumentResult {
  status: "pending" | "success" | "error";
  isFetching: boolean;
  data?: DocumentResponse;
  error?: unknown;
}

/**
 * The read state of one associated document. A query that keeps successful
 * cached data while it refreshes is not a current read, and a query that
 * failed after a refresh is not a usable read either: neither may be
 * presented as the current content of the file.
 */
export function libraryPreviewDocumentState(
  ref: LibraryPreviewDocumentRef,
  result: LibraryPreviewDocumentResult | undefined,
): LibraryPreviewDocumentState {
  if (!result || result.status === "pending") {
    return { documentId: ref.documentId, title: ref.title, status: "pending" };
  }
  if (result.status === "error") {
    return { documentId: ref.documentId, title: ref.title, status: "failed", error: result.error };
  }
  if (result.isFetching) {
    return {
      documentId: ref.documentId,
      title: ref.title,
      status: "refreshing",
      document: result.data,
    };
  }
  return { documentId: ref.documentId, title: ref.title, status: "ready", document: result.data };
}

export interface LibraryPreviewContent {
  /** The readable blocks of every document whose content is available. */
  blocks: LibraryPreviewBlock[];
  /** Documents that were never read. */
  pending: LibraryPreviewDocumentState[];
  /** Documents whose cached content is being refreshed. */
  refreshing: LibraryPreviewDocumentState[];
  /** Documents whose content could not be read. */
  failed: LibraryPreviewDocumentState[];
  /** Whether every associated document is read and current. */
  complete: boolean;
}

/**
 * The readable blocks of the file, in document order and chunk order. A block
 * keeps the reader's stale and empty semantics: such a block is never
 * presented as document content.
 */
export function libraryPreviewBlocks(states: LibraryPreviewDocumentState[]): LibraryPreviewBlock[] {
  const blocks: LibraryPreviewBlock[] = [];
  for (const state of states) {
    if ((state.status !== "ready" && state.status !== "refreshing") || !state.document) {
      continue;
    }
    const documentBlocksForState = documentBlocks(state.document);
    documentBlocksForState.forEach((block, index) => {
      blocks.push({
        block,
        documentId: state.documentId,
        chunkIndex: block.chunk.chunk_index,
        sectionTitle: index === 0 && state.title ? state.title : null,
      });
    });
  }
  return blocks;
}

/**
 * The whole-file content view. A document that is still loading, that is
 * being refreshed or that failed keeps the preview incomplete: the remaining
 * blocks must never be presented as the complete current stored text.
 */
export function libraryPreviewContent(states: LibraryPreviewDocumentState[]): LibraryPreviewContent {
  const pending = states.filter((state) => state.status === "pending");
  const refreshing = states.filter((state) => state.status === "refreshing");
  const failed = states.filter((state) => state.status === "failed");
  return {
    blocks: libraryPreviewBlocks(states),
    pending,
    refreshing,
    failed,
    complete: pending.length === 0 && refreshing.length === 0 && failed.length === 0,
  };
}

/** The read state of the selected file's own detail. */
export interface LibraryPreviewDetailState {
  /** The file the retained detail belongs to. */
  fileId: string | null;
  /** The detail read succeeded and no refresh is running. */
  settled: boolean;
  /** A refresh of the detail is in flight. */
  refreshing: boolean;
  /** The last detail read failed. */
  failed: boolean;
}

export interface LibraryPreviewReadiness {
  /** The documents that may be read right now. */
  refs: LibraryPreviewDocumentRef[];
  /** The retained detail belongs to the selected file. */
  identityMatches: boolean;
  /** The stored content may be read and shown as the current content. */
  ready: boolean;
  /** The detail is being refreshed, so its document set is unverified. */
  refreshing: boolean;
  /** The detail could not be read, so its document set is unknown. */
  failed: boolean;
}

/**
 * Whether the stored content of the selected file may be read and shown.
 *
 * Only a detail that belongs to the selected file and that settled
 * successfully and idle identifies the current document set. Retained detail
 * of a running refresh or of a failed read is unverified, so it issues no
 * document reads and never presents an old association set as the current
 * one.
 */
export function libraryPreviewReadiness(
  selectedFileId: string | null,
  detail: LibraryPreviewDetailState,
  sections: LibraryDocumentSectionPreview[],
): LibraryPreviewReadiness {
  const identityMatches = !!selectedFileId && !!detail.fileId && selectedFileId === detail.fileId;
  if (detail.failed) {
    return { refs: [], identityMatches, ready: false, refreshing: false, failed: true };
  }
  if (!identityMatches) {
    return { refs: [], identityMatches: false, ready: false, refreshing: false, failed: false };
  }
  if (detail.refreshing) {
    return { refs: [], identityMatches: true, ready: false, refreshing: true, failed: false };
  }
  if (detail.settled) {
    return {
      refs: libraryPreviewDocumentRefs(sections),
      identityMatches: true,
      ready: true,
      refreshing: false,
      failed: false,
    };
  }
  return { refs: [], identityMatches: true, ready: false, refreshing: false, failed: false };
}

export function libraryPreviewPageCount(blocks: LibraryPreviewBlock[]): number {
  return documentPageCount(blocks.map((entry) => entry.block));
}

/** The current page, clamped into the available page range. */
export function libraryPreviewCurrentPage(page: number, totalPages: number): number {
  const total = Math.max(1, totalPages);
  return Math.min(Math.max(1, page), total);
}

/** The blocks of one page, ten stored blocks per page as the reader shows. */
export function libraryPreviewPage(
  blocks: LibraryPreviewBlock[],
  page: number,
): LibraryPreviewBlock[] {
  const current = libraryPreviewCurrentPage(page, libraryPreviewPageCount(blocks));
  const start = (current - 1) * DOCUMENT_PAGE_SIZE;
  return blocks.slice(start, start + DOCUMENT_PAGE_SIZE);
}

export interface LibraryPreviewPageState {
  /** The file identity the page belongs to. */
  scope: string;
  page: number;
}

/**
 * The page state of the current file. A different file starts at page one
 * synchronously, so the previous file's page can never be shown with the new
 * file's content. A closed preview has no file, so it clears the reading
 * scope: every fresh open starts at page one, even for the same file.
 */
export function libraryPreviewPageState(
  scope: string,
  stored: LibraryPreviewPageState,
): LibraryPreviewPageState {
  return stored.scope === scope ? stored : { scope, page: 1 };
}
