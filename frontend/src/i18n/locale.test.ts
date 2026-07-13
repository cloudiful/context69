import { beforeEach, describe, expect, it } from "vitest";

import { createAppI18n } from ".";
import { installMockStorage } from "../test-utils/storage";
import { LOCALE_STORAGE_KEY, normalizeAppLocale, resolveInitialLocale } from "./locale";

describe("locale initialization", () => {
  beforeEach(() => {
    installMockStorage();
  });

  it("defaults to english when no locale is stored", () => {
    const i18n = createAppI18n(resolveInitialLocale(window.localStorage));

    expect(i18n.global.locale.value).toBe("en");
  });

  it("restores a stored chinese locale", () => {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");

    const i18n = createAppI18n(resolveInitialLocale(window.localStorage));

    expect(i18n.global.locale.value).toBe("zh-CN");
  });

  it("falls back to english for invalid stored locales", () => {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "fr");

    const i18n = createAppI18n(resolveInitialLocale(window.localStorage));

    expect(i18n.global.locale.value).toBe("en");
  });

  it("normalizes legacy and browser-style locale variants", () => {
    expect(normalizeAppLocale("zh")).toBe("zh-CN");
    expect(normalizeAppLocale("zh_CN")).toBe("zh-CN");
    expect(normalizeAppLocale("zh-Hans")).toBe("zh-CN");
    expect(normalizeAppLocale("en-US")).toBe("en");
  });
});
