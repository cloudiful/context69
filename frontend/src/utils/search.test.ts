import { describe, expect, it } from "vitest";

import { buildSearchPayload, cursorFromQuery, filtersFromQuery, filtersToQuery, groupFolderOptions, loadSearchSession, normalizeSearchFilters, pageFromQuery, sameSearchFilters, saveSearchSession } from "./search";
import { installMockStorage } from "../test-utils/storage";

describe("search utilities", () => {
  it("hydrates filters from route query", () => {
    const filters = filtersFromQuery({
      q: "regulation",
      source: "gov_documents",
      after: "2024-01-01",
      before: "2024-02-01",
      limit: "12",
    });

    expect(filters).toEqual({
      query: "regulation",
      sourceKey: "gov_documents",
      publishedAfter: "2024-01-01",
      publishedBefore: "2024-02-01",
      limit: 12,
      groupPath: "",
      sort: "relevance",
    });
  });

  it("reads the sort URL parameter", () => {
    const filters = filtersFromQuery({
      q: "regulation",
      sort: "date",
    });
    expect(filters.sort).toBe("date");
  });

  it("ignores an invalid sort URL parameter", () => {
    const filters = filtersFromQuery({
      q: "regulation",
      sort: "garbage",
    });
    expect(filters.sort).toBe("relevance");
  });

  it("serializes filters for search request and route query", () => {
    const filters = {
      query: "  policy  ",
      sourceKey: "gov_documents",
      publishedAfter: "",
      publishedBefore: "2025-01-31",
      limit: 99,
    };

    expect(buildSearchPayload(filters)).toEqual({
      query: "policy",
      limit: 50,
      page: 1,
      source_key: "gov_documents",
      published_after: undefined,
      published_before: "2025-01-31",
      group_path: undefined,
      sort: undefined,
    });

    expect(filtersToQuery(filters)).toEqual({
      q: "  policy  ",
      source: "gov_documents",
      after: undefined,
      before: "2025-01-31",
      limit: "99",
      page: undefined,
      group_path: undefined,
      sort: undefined,
    });
  });

  it("serializes the date sort into payload and URL", () => {
    const filters = {
      query: "policy",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
      sort: "date" as const,
    };
    expect(buildSearchPayload(filters).sort).toBe("date");
    expect(filtersToQuery(filters).sort).toBe("date");
    // relevance is the default and should be omitted from the URL.
    const relevance = { ...filters, sort: "relevance" as const };
    expect(filtersToQuery(relevance).sort).toBeUndefined();
  });

  it("handles page param in query and filtersToQuery", () => {
    expect(pageFromQuery({ page: "2" })).toBe(2);
    expect(pageFromQuery({ page: "0" })).toBe(1);
    expect(pageFromQuery({})).toBe(1);
    expect(pageFromQuery({ page: ["3", "4"] })).toBe(3);
    const filters = { query: "a", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const };
    expect(filtersToQuery(filters, 1)).toEqual({ q: "a", source: undefined, after: undefined, before: undefined, limit: undefined, page: undefined, group_path: undefined, sort: undefined });
    expect(filtersToQuery(filters, 3)).toEqual({ q: "a", source: undefined, after: undefined, before: undefined, limit: undefined, page: "3", group_path: undefined, sort: undefined });
    expect(filtersToQuery({ ...filters, publishedAfter: "2025-01-01", publishedBefore: "2025-02-01" }, 2)).toEqual({
      q: "a",
      source: undefined,
      after: "2025-01-01",
      before: "2025-02-01",
      limit: undefined,
      page: "2",
      group_path: undefined,
      sort: undefined,
    });
    expect(filtersToQuery({ ...filters, groupPath: "stock/sgs" }, 1)).toEqual({
      q: "a",
      source: undefined,
      after: undefined,
      before: undefined,
      limit: undefined,
      page: undefined,
      group_path: "stock/sgs",
      sort: undefined,
    });
  });

  it("flattens accessible groups into indented folder options", () => {
    expect(groupFolderOptions([])).toEqual([]);
    const options = groupFolderOptions([
      { group_id: 2, group_key: "sgs", group_path: "stock/sgs-disclosures", name: "SGS", visibility: "private", kind: "shared", created_at: "x", updated_at: "x" },
      { group_id: 1, group_key: "stock", group_path: "stock", name: "Stock", visibility: "private", kind: "shared", created_at: "x", updated_at: "x" },
      { group_id: 3, group_key: "nopath", group_path: "", name: "NoPath", visibility: "private", kind: "shared", created_at: "x", updated_at: "x" },
    ]);
    expect(options).toEqual([
      { value: "stock", label: "stock" },
      { value: "stock/sgs-disclosures", label: "  stock / sgs-disclosures" },
    ]);
  });

  it("compares filters and normalizes correctly, handling local date strings", () => {
    const a = { query: "  test ", sourceKey: "src", publishedAfter: "2025-01-01", publishedBefore: "", limit: 16, sort: "relevance" as const };
    const b = normalizeSearchFilters(a);
    expect(b.query).toBe("test");
    expect(b.limit).toBe(16);
    expect(b.sort).toBe("relevance");
    expect(sameSearchFilters({ query: "x", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const }, { query: "x", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const })).toBe(true);
    expect(sameSearchFilters({ query: "x", sourceKey: "a", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const }, { query: "x", sourceKey: "b", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const })).toBe(false);
    // empty after/before should remain empty, not undefined
    const c = normalizeSearchFilters({ query: "q", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const });
    expect(c.publishedAfter).toBe("");
    expect(c.publishedBefore).toBe("");
    // different sort modes must not be considered equal
    expect(sameSearchFilters({ query: "x", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const }, { query: "x", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "date" as const })).toBe(false);
  });

  it("saves and loads search session for back navigation without polluting URL with large objects", () => {
    const storage = installMockStorage();
    // mock sessionStorage using same mock for simplicity
    Object.defineProperty(window, "sessionStorage", { value: storage, configurable: true });
    const filters = { query: "session test", sourceKey: "src", publishedAfter: "2025-01-01", publishedBefore: "", limit: 16, sort: "relevance" as const };
    saveSearchSession(filters, 3);
    const loaded = loadSearchSession();
    expect(loaded?.filters.query).toBe("session test");
    expect(loaded?.filters.publishedAfter).toBe("2025-01-01");
    expect(loaded?.page).toBe(3);
    // ensure save does not store large objects, only filters+page
    const raw = storage.getItem("context69.search-session");
    expect(raw).not.toContain("chunk_id");
    expect(JSON.parse(raw!).page).toBe(3);
  });

  it("does not persist session when query is empty", () => {
    const storage = installMockStorage();
    Object.defineProperty(window, "sessionStorage", { value: storage, configurable: true });
    saveSearchSession({ query: "   ", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const }, 1);
    expect(loadSearchSession()).toBeNull();
  });

  it("round-trips the cursor URL parameter and keeps page out when a cursor is set", () => {
    expect(cursorFromQuery({})).toBeNull();
    expect(cursorFromQuery({ cursor: "c1:abc" })).toBe("c1:abc");
    expect(cursorFromQuery({ cursor: ["c1:a", "c1:b"] })).toBe("c1:a");

    const filters = { query: "q", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const };
    expect(filtersToQuery(filters, { cursor: "c1:abc" })).toEqual({
      q: "q",
      source: undefined,
      after: undefined,
      before: undefined,
      limit: undefined,
      page: undefined,
      cursor: "c1:abc",
      group_path: undefined,
      sort: undefined,
    });
    expect(filtersToQuery(filters, 2)).toEqual({
      q: "q",
      source: undefined,
      after: undefined,
      before: undefined,
      limit: undefined,
      page: "2",
      cursor: undefined,
      group_path: undefined,
      sort: undefined,
    });
    // Legacy number form stays compatible for callers that never navigate by cursor.
    const legacy = filtersToQuery(filters, 1);
    expect(legacy.page).toBeUndefined();
    expect(legacy.cursor).toBeUndefined();
  });

  it("builds cursor and legacy page payloads", () => {
    const filters = { query: "q", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const };
    expect(buildSearchPayload(filters, { cursor: "c1:8" })).toEqual({
      query: "q",
      limit: 8,
      page: 1,
      cursor: "c1:8",
      source_key: undefined,
      published_after: undefined,
      published_before: undefined,
      group_path: undefined,
      sort: "relevance",
    });
    expect(buildSearchPayload(filters, 2)).toMatchObject({ page: 2, cursor: undefined, sort: "relevance" });
    expect(buildSearchPayload(filters)).toMatchObject({ page: 1, cursor: undefined, sort: "relevance" });
  });

  it("persists and restores the cursor in the search session", () => {
    const storage = installMockStorage();
    Object.defineProperty(window, "sessionStorage", { value: storage, configurable: true });
    const filters = { query: "session cursor", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8, sort: "relevance" as const };
    saveSearchSession(filters, 4, "c1:24");
    const loaded = loadSearchSession();
    expect(loaded?.page).toBe(4);
    expect(loaded?.cursor).toBe("c1:24");
    // Legacy sessions without a cursor still load.
    saveSearchSession(filters, 2);
    expect(loadSearchSession()?.cursor).toBeUndefined();
  });
});
