import { computed } from "vue";

import type { GroupResponse } from "../../services/api";
import type { GroupExplorerEntry } from "../../types/library";

interface UseGroupBrowserEntriesOptions {
  childGroups: () => GroupResponse[];
  libraryEntryCount: () => number;
  query: () => string;
  t: (key: string, params?: Record<string, unknown>) => string;
}

export function useGroupBrowserEntries(options: UseGroupBrowserEntriesOptions) {
  const groupEntries = computed<GroupExplorerEntry[]>(() => options.childGroups().map((group) => ({
    key: `group:${group.group_id}`,
    kind: "group",
    id: group.group_id,
    depth: 0,
    name: group.name,
    parentFolderId: null,
    path: group.group_path ?? group.group_key,
    updatedAt: group.updated_at,
    visibility: group.visibility,
    group,
  })));

  const filteredGroupEntries = computed(() => {
    const query = options.query().trim().toLowerCase();
    if (!query) return groupEntries.value;
    return groupEntries.value.filter((entry) => [
      entry.name,
      entry.path,
      entry.visibility,
      options.t("groups.groupType"),
    ].some((value) => value.toLowerCase().includes(query)));
  });

  const resourceCountLabel = computed(() => options.t("library.resourceCount", {
    count: filteredGroupEntries.value.length + options.libraryEntryCount(),
  }));

  return { filteredGroupEntries, resourceCountLabel };
}
