import type { DocumentChunkResponse, DocumentResponse } from "@/services/api";
import { SEARCH_RETURN_QUERY_KEY, SEARCH_RETURN_QUERY_VALUE } from "@/utils/search-target";

/** The reader shows ten stored blocks per page, matching the baseline reader. */
export const DOCUMENT_PAGE_SIZE = 10;

/**
 * A route id must be a positive, canonical decimal integer. `Number.parseInt`
 * would accept `12abc` and fetch a document the URL does not name.
 */
export function parseDocumentId(raw: string | undefined | null): number | null {
  if (!raw) {
    return null;
  }
  const trimmed = raw.trim();
  if (!/^[1-9]\d*$/.test(trimmed)) {
    return null;
  }
  const value = Number(trimmed);
  return Number.isSafeInteger(value) ? value : null;
}

export interface DocumentBlock {
  chunk: DocumentChunkResponse;
  /** The block text is safe to display only when it matches the header hash. */
  valid: boolean;
  reason: "changed" | "empty" | null;
}

/**
 * Header and chunk reads are independent, so a stale or empty block is
 * observable. Such a block is never rendered as document content.
 */
export function documentBlocks(document: DocumentResponse): DocumentBlock[] {
  return document.chunks.map((chunk) => {
    if (!chunk.text.trim()) {
      return { chunk, valid: false, reason: "empty" };
    }
    if (chunk.record_hash !== document.record_hash) {
      return { chunk, valid: false, reason: "changed" };
    }
    return { chunk, valid: true, reason: null };
  });
}

export function documentPageCount(blocks: DocumentBlock[]): number {
  return Math.max(1, Math.ceil(blocks.length / DOCUMENT_PAGE_SIZE));
}

export function documentPage(blocks: DocumentBlock[], page: number): DocumentBlock[] {
  const current = Math.min(Math.max(1, page), documentPageCount(blocks));
  const start = (current - 1) * DOCUMENT_PAGE_SIZE;
  return blocks.slice(start, start + DOCUMENT_PAGE_SIZE);
}

const ALLOWED_EXTERNAL_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

/**
 * The only external targets the reader may open: the protocols the shared
 * markdown renderer already accepts. Script, data and relative targets are
 * refused instead of being handed to `window.open`.
 */
export function safeExternalUrl(value: string | null | undefined): string | null {
  if (!value) {
    return null;
  }
  try {
    const parsed = new URL(value.trim());
    return ALLOWED_EXTERNAL_PROTOCOLS.has(parsed.protocol) ? parsed.toString() : null;
  } catch {
    return null;
  }
}

/**
 * The in-app target of a library-backed document. It carries the generated
 * library file id in the encoded group path, and keeps the search return marker
 * so the library can offer the way back.
 */
export function documentLibraryTarget(
  document: DocumentResponse,
  fromSearch: boolean,
): string | null {
  if (!document.is_library_file || !document.library_file_id || !document.group_path) {
    return null;
  }
  const params = new URLSearchParams();
  params.set("file", document.library_file_id);
  if (fromSearch) {
    params.set(SEARCH_RETURN_QUERY_KEY, SEARCH_RETURN_QUERY_VALUE);
  }
  return `/groups/${encodeURIComponent(document.group_path)}?${params.toString()}`;
}

export type DocumentErrorKind = "invalid" | "not_found" | "forbidden" | "load_failed";

export function documentErrorKind(error: unknown): DocumentErrorKind {
  const status = typeof error === "object" && error && "status" in error
    ? (error as { status?: unknown }).status
    : undefined;
  if (status === 404) {
    return "not_found";
  }
  if (status === 403) {
    return "forbidden";
  }
  return "load_failed";
}
