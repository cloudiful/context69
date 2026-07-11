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
});
