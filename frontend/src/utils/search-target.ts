import type { SearchHit } from "../services/api";

export function buildSearchTarget(hit: SearchHit) {
  if (hit.is_library_file && hit.library_file_id) {
    return {
      name: "group-overview",
      params: {
        groupPath: hit.group_path,
      },
      query: {
        file: hit.library_file_id,
      },
    };
  }

  return {
    name: "document",
    params: {
      id: hit.document_id,
    },
  };
}
