import { describe, expect, it } from "vitest";

import { visibilityLabel } from "./visibility-label";

function translator(translations: Record<string, string>) {
  return (key: string) => translations[key] ?? key;
}

const ENGLISH = translator({
  "groups.visibilityOptions.private": "Private",
  "groups.visibilityOptions.public": "Public",
});

const CHINESE = translator({
  "groups.visibilityOptions.private": "私有",
  "groups.visibilityOptions.public": "公开",
});

describe("visibilityLabel", () => {
  it("localizes both generated visibility values in English", () => {
    expect(visibilityLabel(ENGLISH, "private")).toBe("Private");
    expect(visibilityLabel(ENGLISH, "public")).toBe("Public");
  });

  it("localizes both generated visibility values in Chinese", () => {
    expect(visibilityLabel(CHINESE, "private")).toBe("私有");
    expect(visibilityLabel(CHINESE, "public")).toBe("公开");
  });

  it("resolves the label through the shared group visibility catalog keys", () => {
    const requested: string[] = [];
    const recording = (key: string) => {
      requested.push(key);
      return key;
    };

    visibilityLabel(recording, "public");
    visibilityLabel(recording, "private");
    expect(requested).toEqual([
      "groups.visibilityOptions.public",
      "groups.visibilityOptions.private",
    ]);
  });

  it("keeps an unknown future value visible instead of guessing a label", () => {
    expect(visibilityLabel(ENGLISH, "future_visibility" as never)).toBe("future_visibility");
    expect(visibilityLabel(CHINESE, "future_visibility" as never)).toBe("future_visibility");
  });

  it("renders the shared placeholder for an absent value", () => {
    expect(visibilityLabel(ENGLISH, null)).toBe("--");
    expect(visibilityLabel(ENGLISH, undefined)).toBe("--");
  });

  it("returns display text only, leaving the enum value untouched", () => {
    const visibility = "public" as const;
    expect(visibilityLabel(ENGLISH, visibility)).toBe("Public");
    // The value used for requests, filters and permission checks is unchanged.
    expect(visibility).toBe("public");
  });
});
