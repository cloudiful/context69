import { describe, expect, it } from "vitest";

import type { LibraryFileSummary, LibraryFolderNode, LibraryIngestStatus } from "@/services/api";

import {
  buildFileExplorerEntry,
  buildFolderExplorerEntry,
  confirmedSourcePolicy,
  selectedFailedFileIds,
  type ExplorerEntry,
} from "./library-model";

const parent: LibraryFolderNode = {
  group_key: "group",
  group_path: "group",
  visibility: "private",
  folder_id: null,
  parent_folder_id: null,
  name: "Root",
  path: "group",
  processing_count: 0,
  children: [],
  files: [],
};

function fileEntry(id: string, status: LibraryIngestStatus): ExplorerEntry {
  const file: LibraryFileSummary = {
    file_id: id,
    group_key: "group",
    group_path: "group",
    visibility: "private",
    folder_id: null,
    filename: `${id}.txt`,
    media_type: "text/plain",
    size_bytes: 1,
    ingest_status: status,
    error_message: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ingested_at: null,
  };
  return buildFileExplorerEntry(file, 0, null, parent.path, parent);
}

const folderEntry = buildFolderExplorerEntry({
  ...parent,
  folder_id: "folder-1",
  parent_folder_id: null,
  name: "Reports",
  path: "group/Reports",
}, 0, null);

describe("selectedFailedFileIds", () => {
  it("retains unavailable selected failed files for skip accounting", () => {
    const entries = [
      fileEntry("retry-me", "failed"),
      fileEntry("missing", "failed"),
      fileEntry("ready", "succeeded"),
    ];
    const selected = new Set(entries.slice(0, 2).map((entry) => entry.key));

    expect(selectedFailedFileIds(entries, selected)).toEqual(["retry-me", "missing"]);
  });

  it("ignores unselected rows, folders, and non-failed statuses", () => {
    const entries = [
      fileEntry("selected", "failed"),
      fileEntry("other", "failed"),
      fileEntry("ready", "succeeded"),
      folderEntry,
    ];

    expect(selectedFailedFileIds(entries, new Set([entries[0].key, entries[2].key, folderEntry.key])))
      .toEqual(["selected"]);
  });
});

describe("confirmedSourcePolicy", () => {
  it("returns retain or release only for an idle successful query", () => {
    expect(confirmedSourcePolicy({
      status: "success",
      fetchStatus: "idle",
      data: { default_source_policy: "retain" },
    })).toBe("retain");
    expect(confirmedSourcePolicy({
      status: "success",
      fetchStatus: "idle",
      data: { default_source_policy: "release_after_processing" },
    })).toBe("release_after_processing");
  });

  it("blocks cached data while success is fetching or paused", () => {
    const data = { default_source_policy: "release_after_processing" } as const;

    expect(confirmedSourcePolicy({ status: "success", fetchStatus: "fetching", data })).toBeNull();
    expect(confirmedSourcePolicy({ status: "success", fetchStatus: "paused", data })).toBeNull();
  });

  it("blocks pending, error, missing data, and missing state", () => {
    const data = { default_source_policy: "retain" } as const;

    expect(confirmedSourcePolicy({ status: "pending", fetchStatus: "idle", data })).toBeNull();
    expect(confirmedSourcePolicy({ status: "error", fetchStatus: "idle", data })).toBeNull();
    expect(confirmedSourcePolicy({ status: "success", fetchStatus: "idle", data: undefined })).toBeNull();
    expect(confirmedSourcePolicy(undefined)).toBeNull();
  });
});
