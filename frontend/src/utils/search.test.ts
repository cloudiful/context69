import { describe, expect, it } from "vitest";

import { buildSearchPayload, filtersFromQuery, filtersToQuery, loadSearchSession, normalizeSearchFilters, pageFromQuery, sameSearchFilters, saveSearchSession } from "./search";
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
    });
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
    });

    expect(filtersToQuery(filters)).toEqual({
      q: "  policy  ",
      source: "gov_documents",
      after: undefined,
      before: "2025-01-31",
      limit: "99",
      page: undefined,
    });
  });

  it("handles page param in query and filtersToQuery", () => {
    expect(pageFromQuery({ page: "2" })).toBe(2);
    expect(pageFromQuery({ page: "0" })).toBe(1);
    expect(pageFromQuery({})).toBe(1);
    expect(pageFromQuery({ page: ["3", "4"] })).toBe(3);
    const filters = { query: "a", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 };
    expect(filtersToQuery(filters, 1)).toEqual({ q: "a", source: undefined, after: undefined, before: undefined, limit: undefined, page: undefined });
    expect(filtersToQuery(filters, 3)).toEqual({ q: "a", source: undefined, after: undefined, before: undefined, limit: undefined, page: "3" });
    expect(filtersToQuery({ ...filters, publishedAfter: "2025-01-01", publishedBefore: "2025-02-01" }, 2)).toEqual({
      q: "a",
      source: undefined,
      after: "2025-01-01",
      before: "2025-02-01",
      limit: undefined,
      page: "2",
    });
  });

  it("compares filters and normalizes correctly, handling local date strings", () => {
    const a = { query: "  test ", sourceKey: "src", publishedAfter: "2025-01-01", publishedBefore: "", limit: 16 };
    const b = normalizeSearchFilters(a);
    expect(b.query).toBe("test");
    expect(b.limit).toBe(16);
    expect(sameSearchFilters({ query: "x", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 }, { query: "x", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 })).toBe(true);
    expect(sameSearchFilters({ query: "x", sourceKey: "a", publishedAfter: "", publishedBefore: "", limit: 8 }, { query: "x", sourceKey: "b", publishedAfter: "", publishedBefore: "", limit: 8 })).toBe(false);
    // empty after/before should remain empty, not undefined
    const c = normalizeSearchFilters({ query: "q", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 });
    expect(c.publishedAfter).toBe("");
    expect(c.publishedBefore).toBe("");
  });

  it("saves and loads search session for back navigation without polluting URL with large objects", () => {
    const storage = installMockStorage();
    // mock sessionStorage using same mock for simplicity
    Object.defineProperty(window, "sessionStorage", { value: storage, configurable: true });
    const filters = { query: "session test", sourceKey: "src", publishedAfter: "2025-01-01", publishedBefore: "", limit: 16 };
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
    saveSearchSession({ query: "   ", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 }, 1);
    expect(loadSearchSession()).toBeNull();
  });
});
