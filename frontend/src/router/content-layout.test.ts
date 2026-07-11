import { describe, expect, it } from "vitest";

import { router } from "./index";

describe("route content layout", () => {
  it("fills the viewport only for workspace pages", () => {
    expect(router.resolve({ name: "search" }).meta.contentLayout).toBe("fill");
    expect(router.resolve({ name: "group-overview", params: { groupPath: "demo" } }).meta.contentLayout).toBe("fill");
  });

  it("keeps settings and management pages content-sized", () => {
    expect(router.resolve({ name: "settings-appearance" }).meta.contentLayout).toBeUndefined();
    expect(router.resolve({ name: "settings-runtime" }).meta.contentLayout).toBeUndefined();
    expect(router.resolve({ name: "groups" }).meta.contentLayout).toBeUndefined();
    expect(router.resolve({ name: "group-members", params: { groupPath: "demo" } }).meta.contentLayout).toBeUndefined();
    expect(router.resolve({ name: "group-settings", params: { groupPath: "demo" } }).meta.contentLayout).toBeUndefined();
  });
});
