import { describe, expect, it } from "vitest";

import { buildSearchPayload, filtersFromQuery, filtersToQuery } from "./search";

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
    });
  });
});
