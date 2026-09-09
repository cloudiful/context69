import { describe, expect, it } from "vitest";

import { installMockStorage } from "../test-utils/storage";
import { SEARCH_HISTORY_STORAGE_KEY } from "./search-history";
import {
  addSearchHistoryEntry,
  readSearchHistory,
  replaySearchEntry,
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

  it("round-trips groupPath (null/empty -> all) and dedupes on it", () => {
    const storage = installMockStorage();

    // groupPath null/undefined normalizes to "" (all directories)
    addSearchHistoryEntry({
      query: "policy",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
      groupPath: null,
    }, storage);
    expect(readSearchHistory(storage)[0].groupPath).toBe("");

    // same query but different groupPath stays as a separate history entry
    addSearchHistoryEntry({
      query: "policy",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
      groupPath: "stock/sgs-disclosures",
    }, storage);
    const afterDifferent = readSearchHistory(storage);
    expect(afterDifferent).toHaveLength(2);
    expect(afterDifferent[0].groupPath).toBe("stock/sgs-disclosures");

    // duplicate query + groupPath dedupes the earlier matching entry
    addSearchHistoryEntry({
      query: "policy",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
      groupPath: "stock/sgs-disclosures",
    }, storage);
    const afterDuplicate = readSearchHistory(storage);
    expect(afterDuplicate).toHaveLength(2);
    expect(afterDuplicate[0].groupPath).toBe("stock/sgs-disclosures");
  });

  it("keeps history entries cursor-free so replaying one always restarts page 1", () => {
    const storage = installMockStorage();
    addSearchHistoryEntry(
      { query: "policy", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, groupPath: "" },
      storage,
    );
    const raw = storage.getItem(SEARCH_HISTORY_STORAGE_KEY);
    expect(raw).not.toContain("cursor");
    expect(raw).not.toContain("page");
    const entries = readSearchHistory(storage);
    expect(entries[0]).toEqual(
      expect.objectContaining({
        query: "policy",
        limit: 8,
      }),
    );
    expect(entries[0]).not.toHaveProperty("cursor");
    expect(entries[0]).not.toHaveProperty("page");
  });

  it("replays an entry as filter-only state without pagination", () => {
    const storage = installMockStorage();
    addSearchHistoryEntry(
      { query: " policy ", sourceKey: "src", publishedAfter: "", publishedBefore: "", limit: 16, groupPath: "stock/sgs", sort: "date" as const },
      storage,
    );
    const [entry] = readSearchHistory(storage);
    const replay = replaySearchEntry(entry);
    expect(replay).toEqual({
      query: "policy",
      sourceKey: "src",
      publishedAfter: "",
      publishedBefore: "",
      limit: 16,
      groupPath: "stock/sgs",
      sort: "date",
    });
    expect(replay).not.toHaveProperty("cursor");
    expect(replay).not.toHaveProperty("page");
    expect(replay).not.toHaveProperty("savedAt");
  });

  it("drops an invalid stored sort and defaults to relevance", () => {
    const storage = installMockStorage();
    storage.setItem(
      "context69.search-history",
      JSON.stringify([{ query: "policy", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, groupPath: "", sort: "bogus", savedAt: new Date().toISOString() }]),
    );
    const [entry] = readSearchHistory(storage);
    expect(entry.sort).toBe("relevance");
  });
});
