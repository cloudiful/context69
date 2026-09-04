import { describe, expect, it } from "vitest";

import { installMockStorage } from "../test-utils/storage";
import {
  addSearchHistoryEntry,
  readSearchHistory,
} from "./search-history";

describe("search history utils", () => {
  it("stores normalized entries and dedupes", () => {
    const storage = installMockStorage();

    addSearchHistoryEntry({
      query: "  policy  ",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    }, storage);
    addSearchHistoryEntry({
      query: "policy",
      sourceKey: "gov_documents",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    }, storage);
    addSearchHistoryEntry({
      query: "policy",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    }, storage);

    const entries = readSearchHistory(storage);
    expect(entries).toHaveLength(2);
    expect(entries[0]).toEqual(
      expect.objectContaining({
        query: "policy",
        sourceKey: "",
      }),
    );
    expect(entries[1]).toEqual(
      expect.objectContaining({
        query: "policy",
        sourceKey: "gov_documents",
      }),
    );
  });

  it("preserves date range and limit in history and keeps query as readable string", () => {
    const storage = installMockStorage();
    addSearchHistoryEntry({
      query: "date test",
      sourceKey: "src",
      publishedAfter: "2025-01-01",
      publishedBefore: "2025-02-15",
      limit: 16,
    }, storage);
    const entries = readSearchHistory(storage);
    expect(entries[0]).toEqual(expect.objectContaining({
      query: "date test",
      sourceKey: "src",
      publishedAfter: "2025-01-01",
      publishedBefore: "2025-02-15",
      limit: 16,
    }));
    // ensure query is string, not object, and no [object Object] serialization
    expect(typeof entries[0].query).toBe("string");
    expect(entries[0].query).not.toBe("[object Object]");
    expect(JSON.stringify(entries[0])).not.toContain("[object Object]");
  });

  it("never stores entry when query is object stringified", () => {
    const storage = installMockStorage();
    // simulate bug where [object Object] would be stored: persistence must reject it
    addSearchHistoryEntry({
      query: "[object Object]",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    }, storage);
    expect(readSearchHistory(storage)).toHaveLength(0);
    // valid entry with real query still stores correctly
    addSearchHistoryEntry({
      query: "real",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
    }, storage);
    expect(readSearchHistory(storage)[0].query).toBe("real");
    expect(JSON.stringify(readSearchHistory(storage))).not.toContain("[object Object]");
  });
});
