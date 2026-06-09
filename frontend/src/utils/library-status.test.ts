import { describe, expect, it, vi } from "vitest";

import { libraryStatusLabel, libraryStatusSeverity } from "./library-status";

describe("library-status", () => {
  it("maps each library status to the expected severity", () => {
    expect(libraryStatusSeverity("pending")).toBe("secondary");
    expect(libraryStatusSeverity("running")).toBe("warn");
    expect(libraryStatusSeverity("succeeded")).toBe("success");
    expect(libraryStatusSeverity("failed")).toBe("danger");
  });

  it("builds translated status labels", () => {
    const t = vi.fn((key: string) => `translated:${key}`);

    expect(libraryStatusLabel(t, "running")).toBe("translated:library.status.running");
    expect(t).toHaveBeenCalledWith("library.status.running");
  });
});
