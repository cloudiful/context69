import { describe, expect, it } from "vitest";

import {
  LIBRARY_DEPENDENCY_KEYS,
  canonicalLibraryDependencyKey,
  libraryDependencyLabel,
  libraryStatusLabel,
  libraryStatusSeverity,
} from "./library-status";

const translate = (key: string) => key;

describe("libraryStatusSeverity", () => {
  it("maps ingest statuses to UI severities", () => {
    expect(libraryStatusSeverity("succeeded")).toBe("success");
    expect(libraryStatusSeverity("failed")).toBe("error");
    expect(libraryStatusSeverity("running")).toBe("warning");
    expect(libraryStatusSeverity("pending")).toBe("neutral");
    expect(libraryStatusSeverity("cancelled")).toBe("neutral");
  });
});

describe("libraryStatusLabel", () => {
  it("resolves the localized label for an ingest status", () => {
    expect(libraryStatusLabel(translate, "succeeded")).toBe("library.status.succeeded");
  });
});

describe("canonicalLibraryDependencyKey", () => {
  it("returns null for empty input", () => {
    expect(canonicalLibraryDependencyKey(null)).toBeNull();
    expect(canonicalLibraryDependencyKey("")).toBeNull();
  });

  it("returns the canonical key unchanged", () => {
    expect(canonicalLibraryDependencyKey("s3")).toBe("s3");
    expect(canonicalLibraryDependencyKey("docling")).toBe("docling");
    expect(canonicalLibraryDependencyKey("embedding")).toBe("embedding");
    expect(canonicalLibraryDependencyKey("qdrant")).toBe("qdrant");
  });

  it("maps the legacy embedding_vector alias to embedding", () => {
    expect(canonicalLibraryDependencyKey("embedding_vector")).toBe("embedding");
  });

  it("returns null for unrecognized keys so callers can render the raw value", () => {
    expect(canonicalLibraryDependencyKey("custom_storage")).toBeNull();
    expect(canonicalLibraryDependencyKey("unknown_future_key")).toBeNull();
  });

  it("covers every canonical key declared by the UI", () => {
    for (const key of LIBRARY_DEPENDENCY_KEYS) {
      expect(canonicalLibraryDependencyKey(key)).toBe(key);
    }
  });
});

describe("libraryDependencyLabel", () => {
  it("returns the localized label for each canonical dependency", () => {
    expect(libraryDependencyLabel(translate, "s3")).toBe("processingQueue.dependencies.s3");
    expect(libraryDependencyLabel(translate, "docling")).toBe("processingQueue.dependencies.docling");
    expect(libraryDependencyLabel(translate, "embedding")).toBe("processingQueue.dependencies.embedding");
    expect(libraryDependencyLabel(translate, "qdrant")).toBe("processingQueue.dependencies.qdrant");
  });

  it("canonicalizes legacy embedding_vector to the embedding label", () => {
    expect(libraryDependencyLabel(translate, "embedding_vector")).toBe("processingQueue.dependencies.embedding");
  });

  it("keeps unknown dependency keys visible as their raw value", () => {
    expect(libraryDependencyLabel(translate, "custom_storage")).toBe("custom_storage");
    expect(libraryDependencyLabel(translate, "unknown_future_key")).toBe("unknown_future_key");
  });

  it("falls back to a placeholder for empty input", () => {
    expect(libraryDependencyLabel(translate, null)).toBe("--");
    expect(libraryDependencyLabel(translate, "")).toBe("--");
  });
});