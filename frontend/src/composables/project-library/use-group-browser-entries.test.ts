import { describe, expect, it } from "vitest";
import { ref } from "vue";

import type { GroupResponse } from "../../services/api";
import { useGroupBrowserEntries } from "./use-group-browser-entries";

const childGroups: GroupResponse[] = [
  {
    group_id: 7,
    group_key: "disclosures",
    group_path: "stock/disclosures",
    parent_group_path: "stock",
    name: "Disclosures",
    visibility: "private",
    kind: "shared",
    current_role: "owner",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-02T00:00:00Z",
  },
];

describe("useGroupBrowserEntries", () => {
  it("maps child groups into root browser entries and includes them in search counts", () => {
    const query = ref("");
    const state = useGroupBrowserEntries({
      childGroups: () => childGroups,
      libraryEntryCount: () => 2,
      query: () => query.value,
      t: (key, params) => key === "groups.groupType" ? "Group" : `${String(params?.count)} items`,
    });

    expect(state.filteredGroupEntries.value).toMatchObject([
      {
        key: "group:7",
        kind: "group",
        depth: 0,
        name: "Disclosures",
        path: "stock/disclosures",
        visibility: "private",
      },
    ]);
    expect(state.resourceCountLabel.value).toBe("3 items");

    query.value = "group";
    expect(state.filteredGroupEntries.value).toHaveLength(1);

    query.value = "missing";
    expect(state.filteredGroupEntries.value).toHaveLength(0);
    expect(state.resourceCountLabel.value).toBe("2 items");
  });
});
