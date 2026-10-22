import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "../test-utils/storage";

describe("locale catalog wiring", () => {
  beforeEach(() => {
    vi.resetModules();
    installMockStorage();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("defaults to english when no locale is stored", async () => {
    const { i18n } = await import("./i18n");

    expect(i18n.language).toBe("en");
    expect(i18n.t("nav.search")).toBe("Search");
  });

  it("restores a stored chinese locale over the shared catalog", async () => {
    window.localStorage.setItem("context69.locale", "zh-CN");
    const { i18n } = await import("./i18n");

    expect(i18n.language).toBe("zh-CN");
    expect(i18n.t("nav.search")).toBe("搜索");
  });

  it("falls back to english for an invalid stored locale", async () => {
    window.localStorage.setItem("context69.locale", "fr");
    const { i18n } = await import("./i18n");

    expect(i18n.language).toBe("en");
  });

  it("interpolates catalog values unchanged", async () => {
    const { i18n } = await import("./i18n");

    expect(i18n.t("document.chunkLabel", { index: 2 })).toBe("Content Block 2");
  });
});
