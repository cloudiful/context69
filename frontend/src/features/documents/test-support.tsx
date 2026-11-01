import { vi } from "vitest";

import { openapiClient } from "@/services/openapi-client";
import type {
  DocumentChunkResponse,
  DocumentResponse,
  DocumentResultResponse,
  ListDocumentResultsResponse,
} from "@/services/api";

export function chunkFixture(overrides: Partial<DocumentChunkResponse> = {}): DocumentChunkResponse {
  return {
    chunk_id: "chunk-1",
    chunk_index: 0,
    record_hash: "hash-42",
    text: "First stored block.",
    ...overrides,
  };
}

/**
 * Builds a fixture from a JSON literal so nested values the generated schema
 * narrows (such as `metadata_json`) stay realistic without a handwritten DTO.
 */
export function documentFrom(json: string): DocumentResponse {
  return JSON.parse(json) as DocumentResponse;
}

export function documentFixture(overrides: Partial<DocumentResponse> = {}): DocumentResponse {
  return {
    document_id: 42,
    source_key: "gov_documents",
    group_key: "alpha",
    group_path: "stock/alpha",
    external_id: "ext-42",
    record_hash: "hash-42",
    visibility: "private",
    title: "Cyber Policy Update",
    summary: "Stored summary about cyber policy.",
    source_uri: "https://example.test/doc",
    published_at: "2026-01-01",
    updated_at: "2026-01-02T08:30:00Z",
    metadata_json: {},
    library_path: "Alpha Research/Reports",
    library_section_label: "Body",
    is_library_file: false,
    library_file_id: null,
    chunks: [chunkFixture()],
    ...overrides,
  };
}

export interface RecordedDocumentCall {
  documentId: number;
  signal?: AbortSignal;
}

export interface DocumentFailure {
  status: number;
  message: string;
}

export interface DocumentBackendOptions {
  document?: DocumentResponse | ((documentId: number) => DocumentResponse);
  results?: ListDocumentResultsResponse | ((documentId: number, query?: { after_id?: number; current_only?: boolean; latest_only?: boolean }) => ListDocumentResultsResponse);
  resultsFailure?: DocumentFailure;
  failure?: DocumentFailure;
  /** Keeps the request pending until `resolve` is called. */
  deferred?: boolean;
}

export function documentResultFixture(
  overrides: Partial<DocumentResultResponse> = {},
): DocumentResultResponse {
  return {
    id: 1,
    document_id: 42,
    author_user_id: 7,
    producer: "stock.news",
    result_kind: "sentiment.v1",
    schema_ref: "stock.news/sentiment@1",
    source_record_hash: "hash-42",
    idempotency_key: "run-1",
    result: { direction: "positive" },
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

export function documentResultsPage(
  items: DocumentResultResponse[] = [],
  overrides: Partial<ListDocumentResultsResponse> = {},
): ListDocumentResultsResponse {
  return {
    source_record_hash: "hash-42",
    items,
    next_after_id: null,
    ...overrides,
  };
}

/**
 * Records generated document reads and answers them locally. Nothing here
 * touches the network: the generated client call is the only seam.
 */
export function installDocumentBackend(options: DocumentBackendOptions = {}) {
  const calls: RecordedDocumentCall[] = [];
  const pending: Array<() => void> = [];
  let failure = options.failure;
  let resultsFailure = options.resultsFailure;

  vi.spyOn(openapiClient, "GET").mockImplementation(((path: string, init?: {
    params?: {
      path?: { document_id?: number };
      query?: { current_only?: boolean; latest_only?: boolean; after_id?: number };
    };
    signal?: AbortSignal;
  }) => {
    const resultsMatch = /^\/v1\/groups\/by-path\/\{group_path\}\/documents\/\{document_id\}\/results$/.test(path);
    if (resultsMatch) {
      if (resultsFailure) {
        return Promise.resolve({
          error: { message: resultsFailure.message },
          response: { ok: false, status: resultsFailure.status },
        });
      }
      const documentId = Number(init?.params?.path?.document_id);
      const value = typeof options.results === "function"
        ? options.results(documentId, init?.params?.query)
        : options.results ?? documentResultsPage([], { source_record_hash: "hash-42" });
      return Promise.resolve({ data: value, response: { ok: true, status: 200 } });
    }
    if (path !== "/v1/documents/{document_id}") {
      return Promise.resolve({ error: { message: "unmocked" }, response: { ok: false, status: 404 } });
    }
    const documentId = Number(init?.params?.path?.document_id);
    calls.push({ documentId, signal: init?.signal });

    const respond = () => {
      if (!failure) {
        const value = typeof options.document === "function"
          ? options.document(documentId)
          : options.document ?? documentFixture({ document_id: documentId });
        return Promise.resolve({ data: value, response: { ok: true, status: 200 } });
      }
      return Promise.resolve({
        error: { message: failure.message },
        response: { ok: false, status: failure.status },
      });
    };

    if (!options.deferred) {
      return respond();
    }
    return new Promise((resolve) => {
      pending.push(() => resolve(respond()));
    });
  }) as never);

  return {
    calls,
    release(count = pending.length) {
      for (const resolve of pending.splice(0, count)) {
        resolve();
      }
    },
    setFailure(next: DocumentFailure | null) {
      failure = next ?? undefined;
    },
    setResultsFailure(next: DocumentFailure | null) {
      resultsFailure = next ?? undefined;
    },
  };
}
