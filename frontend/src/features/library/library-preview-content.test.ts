import { describe, expect, it } from "vitest";

import type {
  DocumentChunkResponse,
  DocumentResponse,
  LibraryDocumentSectionPreview,
} from "@/services/api";

import {
  libraryPreviewContent,
  libraryPreviewCurrentPage,
  libraryPreviewDocumentRefs,
  libraryPreviewDocumentState,
  libraryPreviewPage,
  libraryPreviewPageCount,
  libraryPreviewPageState,
  libraryPreviewReadiness,
  type LibraryPreviewDocumentState,
} from "./library-preview-content";

const RECORD_HASH = "hash-1";

function chunk(index: number, text: string, recordHash = RECORD_HASH): DocumentChunkResponse {
  return {
    chunk_id: `chunk-${index}`,
    chunk_index: index,
    record_hash: recordHash,
    text,
  };
}

function document(
  documentId: number,
  texts: string[],
  options: { recordHash?: string; title?: string } = {},
): DocumentResponse {
  return {
    chunks: texts.map((text, index) => chunk(index, text, options.recordHash ?? RECORD_HASH)),
    document_id: documentId,
    external_id: `external-${documentId}`,
    group_key: "group",
    group_path: "group",
    is_library_file: true,
    library_file_id: `file-${documentId}`,
    library_path: "group/file.pdf",
    record_hash: options.recordHash ?? RECORD_HASH,
    source_key: "source",
    source_uri: "https://example.invalid/file.pdf",
    title: options.title ?? `Document ${documentId}`,
    updated_at: "2026-01-01T00:00:00Z",
    visibility: "private",
  };
}

function section(
  documentId: number,
  options: { sectionKey?: string; sortOrder?: number; title?: string } = {},
): LibraryDocumentSectionPreview {
  return {
    document_id: documentId,
    preview_text: "preview",
    section_key: options.sectionKey ?? `section-${documentId}`,
    section_label: options.title ?? "Section",
    sort_order: options.sortOrder ?? documentId,
    title: options.title ?? `Section ${documentId}`,
  };
}

function readyState(documentId: number, documentResponse: DocumentResponse): LibraryPreviewDocumentState {
  return {
    documentId,
    title: `Section ${documentId}`,
    status: "ready",
    document: documentResponse,
  };
}

describe("libraryPreviewDocumentRefs", () => {
  it("keeps the declared section order and reads a repeated document once", () => {
    const refs = libraryPreviewDocumentRefs([
      section(2, { sortOrder: 2 }),
      section(1, { sortOrder: 1 }),
      section(2, { sortOrder: 2, sectionKey: "section-2b" }),
    ]);

    expect(refs.map((ref) => ref.documentId)).toEqual([1, 2]);
    expect(refs[1].title).toBe("Section 2");
  });

  it("returns nothing for a file without sections", () => {
    expect(libraryPreviewDocumentRefs([])).toEqual([]);
  });
});

describe("libraryPreviewContent", () => {
  it("keeps the order of several documents and titles every document once", () => {
    const content = libraryPreviewContent([
      readyState(1, document(1, ["a", "b"])),
      readyState(2, document(2, ["c"])),
    ]);

    expect(content.blocks.map((entry) => entry.block.chunk.text)).toEqual(["a", "b", "c"]);
    expect(content.blocks.map((entry) => entry.sectionTitle)).toEqual(["Section 1", null, "Section 2"]);
    expect(content.complete).toBe(true);
    expect(content.pending).toEqual([]);
    expect(content.failed).toEqual([]);
  });

  it("never renders a stale or empty block as content", () => {
    const stale = document(1, ["kept", "", "changed"]);
    stale.chunks[2].record_hash = "hash-other";
    const content = libraryPreviewContent([readyState(1, stale)]);

    expect(content.blocks.map((entry) => entry.block.valid)).toEqual([true, false, false]);
    expect(content.blocks[1].block.reason).toBe("empty");
    expect(content.blocks[2].block.reason).toBe("changed");
  });

  it("does not present pending or failed documents as complete content", () => {
    const pending = libraryPreviewContent([
      readyState(1, document(1, ["a"])),
      { documentId: 2, title: "Section 2", status: "pending" },
    ]);
    expect(pending.complete).toBe(false);
    expect(pending.pending.map((state) => state.documentId)).toEqual([2]);
    expect(pending.blocks.map((entry) => entry.block.chunk.text)).toEqual(["a"]);

    const failed = libraryPreviewContent([
      readyState(1, document(1, ["a"])),
      { documentId: 2, title: "Section 2", status: "failed", error: new Error("denied") },
    ]);
    expect(failed.complete).toBe(false);
    expect(failed.failed.map((state) => state.documentId)).toEqual([2]);
    expect(failed.blocks.map((entry) => entry.block.chunk.text)).toEqual(["a"]);
  });

  it("keeps the cached copy of a refreshing document but never calls it complete", () => {
    const content = libraryPreviewContent([
      readyState(1, document(1, ["a"])),
      { documentId: 2, title: "Section 2", status: "refreshing", document: document(2, ["b"]) },
    ]);

    expect(content.blocks.map((entry) => entry.block.chunk.text)).toEqual(["a", "b"]);
    expect(content.refreshing.map((state) => state.documentId)).toEqual([2]);
    expect(content.complete).toBe(false);
  });
});

describe("libraryPreviewDocumentState", () => {
  const ref = { documentId: 7, sectionKey: "section-7", title: "Section 7", sortOrder: 1 };

  it("treats a settled read as ready", () => {
    const state = libraryPreviewDocumentState(ref, {
      status: "success",
      isFetching: false,
      data: document(7, ["a"]),
    });

    expect(state.status).toBe("ready");
    expect(state.document?.chunks).toHaveLength(1);
  });

  it("treats cached data of a running refresh as refreshing, not ready", () => {
    const state = libraryPreviewDocumentState(ref, {
      status: "success",
      isFetching: true,
      data: document(7, ["a"]),
    });

    expect(state.status).toBe("refreshing");
    expect(state.document?.chunks).toHaveLength(1);
  });

  it("treats a failed read as failed even when it kept its data", () => {
    const state = libraryPreviewDocumentState(ref, {
      status: "error",
      isFetching: false,
      data: document(7, ["a"]),
      error: new Error("denied"),
    });

    expect(state.status).toBe("failed");
    expect(state.error).toBeInstanceOf(Error);
  });

  it("treats an unread document as pending", () => {
    expect(libraryPreviewDocumentState(ref, undefined).status).toBe("pending");
    expect(libraryPreviewDocumentState(ref, {
      status: "pending",
      isFetching: true,
    }).status).toBe("pending");
  });
});

describe("libraryPreviewReadiness", () => {
  const sections = [section(1), section(2)];

  it("reads the documents of a settled detail of the selected file", () => {
    const readiness = libraryPreviewReadiness("file-1", {
      fileId: "file-1",
      settled: true,
      refreshing: false,
      failed: false,
    }, sections);

    expect(readiness.ready).toBe(true);
    expect(readiness.identityMatches).toBe(true);
    expect(readiness.refs.map((ref) => ref.documentId)).toEqual([1, 2]);
  });

  it("issues no reads while the detail of the selected file is refreshing", () => {
    const readiness = libraryPreviewReadiness("file-1", {
      fileId: "file-1",
      settled: false,
      refreshing: true,
      failed: false,
    }, sections);

    expect(readiness.ready).toBe(false);
    expect(readiness.refreshing).toBe(true);
    expect(readiness.refs).toEqual([]);
  });

  it("is not complete after a failed detail read", () => {
    const readiness = libraryPreviewReadiness("file-1", {
      fileId: "file-1",
      settled: false,
      refreshing: false,
      failed: true,
    }, sections);

    expect(readiness.ready).toBe(false);
    expect(readiness.failed).toBe(true);
    expect(readiness.refs).toEqual([]);
  });

  it("issues no reads for a detail of another file", () => {
    const readiness = libraryPreviewReadiness("file-2", {
      fileId: "file-1",
      settled: true,
      refreshing: false,
      failed: false,
    }, sections);

    expect(readiness.ready).toBe(false);
    expect(readiness.identityMatches).toBe(false);
    expect(readiness.refs).toEqual([]);
  });

  it("issues no reads before the detail of the selected file arrived", () => {
    const readiness = libraryPreviewReadiness("file-1", {
      fileId: null,
      settled: false,
      refreshing: false,
      failed: false,
    }, sections);

    expect(readiness.ready).toBe(false);
    expect(readiness.refs).toEqual([]);
  });
});

describe("libraryPreviewPage", () => {
  it("paginates ten blocks per page and keeps a partial last page", () => {
    const blocks = libraryPreviewContent([
      readyState(1, document(1, Array.from({ length: 11 }, (_, index) => `block-${index}`))),
    ]).blocks;

    expect(libraryPreviewPageCount(blocks)).toBe(2);
    expect(libraryPreviewPage(blocks, 1)).toHaveLength(10);
    expect(libraryPreviewPage(blocks, 1)[0].block.chunk.text).toBe("block-0");
    expect(libraryPreviewPage(blocks, 2)).toHaveLength(1);
    expect(libraryPreviewPage(blocks, 2)[0].block.chunk.text).toBe("block-10");
  });

  it("shows a single page for zero and one block", () => {
    const empty = libraryPreviewContent([]).blocks;
    expect(libraryPreviewPageCount(empty)).toBe(1);
    expect(libraryPreviewPage(empty, 1)).toEqual([]);

    const single = libraryPreviewContent([readyState(1, document(1, ["only"]))]).blocks;
    expect(libraryPreviewPageCount(single)).toBe(1);
    expect(libraryPreviewPage(single, 1)).toHaveLength(1);
  });

  it("paginates exactly ten blocks into one full page", () => {
    const blocks = libraryPreviewContent([
      readyState(1, document(1, Array.from({ length: 10 }, (_, index) => `block-${index}`))),
    ]).blocks;

    expect(libraryPreviewPageCount(blocks)).toBe(1);
    expect(libraryPreviewPage(blocks, 1)).toHaveLength(10);
    expect(libraryPreviewPage(blocks, 1)[9].block.chunk.text).toBe("block-9");
    expect(libraryPreviewPage(blocks, 2)).toHaveLength(10);
  });

  it("paginates exactly twenty blocks into two full pages", () => {
    const blocks = libraryPreviewContent([
      readyState(1, document(1, Array.from({ length: 20 }, (_, index) => `block-${index}`))),
    ]).blocks;

    expect(libraryPreviewPageCount(blocks)).toBe(2);
    expect(libraryPreviewPage(blocks, 1)).toHaveLength(10);
    expect(libraryPreviewPage(blocks, 2)).toHaveLength(10);
    expect(libraryPreviewPage(blocks, 2)[9].block.chunk.text).toBe("block-19");
  });

  it("clamps a page outside the available range", () => {
    const blocks = libraryPreviewContent([
      readyState(1, document(1, Array.from({ length: 11 }, (_, index) => `block-${index}`))),
    ]).blocks;

    expect(libraryPreviewCurrentPage(0, 2)).toBe(1);
    expect(libraryPreviewCurrentPage(5, 2)).toBe(2);
    expect(libraryPreviewCurrentPage(-3, 2)).toBe(1);
    expect(libraryPreviewPage(blocks, 5)).toHaveLength(1);
    expect(libraryPreviewPage(blocks, 0)).toHaveLength(10);
  });
});

describe("libraryPreviewPageState", () => {
  it("keeps the page of the same file", () => {
    const stored = { scope: "file-1", page: 3 };
    expect(libraryPreviewPageState("file-1", stored)).toBe(stored);
  });

  it("starts a different file at page one", () => {
    expect(libraryPreviewPageState("file-2", { scope: "file-1", page: 3 })).toEqual({
      scope: "file-2",
      page: 1,
    });
  });

  it("keeps the page while the same file is selected and refreshed", () => {
    const stored = { scope: "file-1", page: 2 };
    expect(libraryPreviewPageState("file-1", stored)).toBe(stored);
  });

  it("starts a reopened file at page one after the preview was closed", () => {
    const blocks = libraryPreviewContent([
      readyState(1, document(1, Array.from({ length: 11 }, (_, index) => `block-${index}`))),
    ]).blocks;
    expect(libraryPreviewPageCount(blocks)).toBe(2);

    // The file is opened and read to its second page.
    let state = libraryPreviewPageState("file-1", { scope: "", page: 1 });
    expect(state).toEqual({ scope: "file-1", page: 1 });
    state = { scope: state.scope, page: 2 };
    expect(libraryPreviewPage(blocks, state.page)).toHaveLength(1);

    // The detail of the still selected file refreshes: the page is kept.
    expect(libraryPreviewPageState("file-1", state)).toBe(state);

    // The preview is closed: no file is selected, so the reading scope and the
    // page are cleared.
    state = libraryPreviewPageState("", state);
    expect(state).toEqual({ scope: "", page: 1 });

    // The same file is opened again: the fresh open starts at page one.
    state = libraryPreviewPageState("file-1", state);
    expect(state).toEqual({ scope: "file-1", page: 1 });
    expect(libraryPreviewPage(blocks, state.page)).toHaveLength(10);
    expect(libraryPreviewPage(blocks, state.page)[0].block.chunk.text).toBe("block-0");
  });
});
