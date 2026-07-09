import type { LibraryFileSummary, LibraryFolderNode, LibraryIngestStatus } from "../services/api";

export interface FolderTreeNode {
  key: string;
  label: string;
  data: {
    folder: LibraryFolderNode;
    countsLabel: string;
  };
  children?: FolderTreeNode[];
  leaf?: boolean;
}

interface ExplorerBaseEntry {
  key: string;
  name: string;
  depth: number;
  parentFolderId: string | null;
  path: string;
  updatedAt: string | null;
}

export interface FolderExplorerEntry extends ExplorerBaseEntry {
  kind: "folder";
  id: string | null;
  childFolderCount: number;
  fileCount: number;
  isSourceFolder: boolean;
  isSourceRecordsFolder: boolean;
  processingCount: number;
  folder: LibraryFolderNode;
}

export interface FileExplorerEntry extends ExplorerBaseEntry {
  kind: "file";
  id: string;
  mediaType: string;
  sizeBytes: number;
  ingestStatus: LibraryIngestStatus;
  errorMessage: string | null;
  isSourceConfigFile: boolean;
  isSourceRecordFile: boolean;
  file: LibraryFileSummary;
}

export type ExplorerEntry = FolderExplorerEntry | FileExplorerEntry;

export interface FolderSummary {
  childFolderCount: number;
  fileCount: number;
  isSourceFolder: boolean;
  isSourceRecordsFolder: boolean;
  name: string;
  path: string;
  processingCount: number;
}
