import { describe, expect, it } from "vitest";

import {
  DOCUMENT_PAGE_SIZE,
  documentBlocks,
  documentErrorKind,
  documentLibraryTarget,
  documentPage,
  documentPageCount,
  parseDocumentId,
  safeExternalUrl,
} from "./document-model";
import { chunkFixture, documentFixture } from "./test-support";

describe("document model", () => {
  it("accepts only canonical positive document ids", () => {
    expect(parseDocumentId("42")).toBe(42);
    expect(parseDocumentId(" 7 ")).toBe(7);
    expect(parseDocumentId("0")).toBeNull();
    expect(parseDocumentId("012")).toBeNull();
    expect(parseDocumentId("12abc")).toBeNull();
    expect(parseDocumentId("-1")).toBeNull();
    expect(parseDocumentId("1.5")).toBeNull();
    expect(parseDocumentId("99999999999999999999")).toBeNull();
    expect(parseDocumentId(undefined)).toBeNull();
  });

  it("rejects empty and hash-mismatched blocks but keeps their position", () => {
    const document = documentFixture({
      chunks: [
        chunkFixture({ chunk_id: "ok", chunk_index: 0 }),
        chunkFixture({ chunk_id: "empty", chunk_index: 1, text: "   " }),
        chunkFixture({ chunk_id: "changed", chunk_index: 2, record_hash: "older-hash" }),
      ],
    });

    const blocks = documentBlocks(document);

    expect(blocks.map((block) => block.valid)).toEqual([true, false, false]);
    expect(blocks.map((block) => block.reason)).toEqual([null, "empty", "changed"]);
  });

  it("pages ten blocks at a time and clamps an out-of-range page", () => {
    const document = documentFixture({
      chunks: Array.from({ length: 23 }, (_, index) => chunkFixture({
        chunk_id: `chunk-${index}`,
        chunk_index: index,
        text: `block-${index + 1}`,
      })),
    });
    const blocks = documentBlocks(document);

    expect(DOCUMENT_PAGE_SIZE).toBe(10);
    expect(documentPageCount(blocks)).toBe(3);
    expect(documentPage(blocks, 1)).toHaveLength(10);
    expect(documentPage(blocks, 3).map((block) => block.chunk.text)).toEqual([
      "block-21",
      "block-22",
      "block-23",
    ]);
    expect(documentPage(blocks, 99)).toHaveLength(3);
    expect(documentPage(blocks, 0)[0].chunk.chunk_index).toBe(0);
  });

  it("allows only the shared renderer protocols for external targets", () => {
    expect(safeExternalUrl("https://example.test/doc")).toBe("https://example.test/doc");
    expect(safeExternalUrl("http://example.test/doc")).toBe("http://example.test/doc");
    expect(safeExternalUrl("mailto:team@example.test")).toBe("mailto:team@example.test");
    expect(safeExternalUrl("javascript:alert(1)")).toBeNull();
    expect(safeExternalUrl("data:text/html;base64,PHNjcmlwdD4=")).toBeNull();
    expect(safeExternalUrl("/groups/stock")).toBeNull();
    expect(safeExternalUrl("")).toBeNull();
    expect(safeExternalUrl(null)).toBeNull();
  });

  it("builds a library target from generated ids and preserves the search marker", () => {
    const library = documentFixture({
      is_library_file: true,
      library_file_id: "file-42",
      group_path: "stock/alpha",
    });

    expect(documentLibraryTarget(library, false)).toBe(
      "/groups/stock%2Falpha?file=file-42",
    );
    expect(documentLibraryTarget(library, true)).toBe(
      "/groups/stock%2Falpha?file=file-42&from=search",
    );
    expect(documentLibraryTarget(documentFixture(), true)).toBeNull();
  });

  it("maps status codes to reader states", () => {
    expect(documentErrorKind({ status: 404 })).toBe("not_found");
    expect(documentErrorKind({ status: 403 })).toBe("forbidden");
    expect(documentErrorKind({ status: 500 })).toBe("load_failed");
    expect(documentErrorKind(new Error("offline"))).toBe("load_failed");
  });
});
