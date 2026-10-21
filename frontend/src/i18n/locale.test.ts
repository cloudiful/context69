import { beforeEach, describe, expect, it } from "vitest";

import { createAppI18n } from ".";
import { installMockStorage } from "../test-utils/storage";
import {
  LOCALE_STORAGE_KEY,
  normalizeAppLocale,
  persistLocale,
  persistLocaleOutcome,
  readStoredLocale,
  resolveInitialLocale,
} from "./locale";

type DenialMode = "getter" | "getItem" | "setItem";

function installDeniedStorage(mode: DenialMode) {
  const denied = () => {
    throw new DOMException("Storage denied", "SecurityError");
  };

  if (mode === "getter") {
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get: denied,
    });
    return;
  }

  const storage = installMockStorage();
  if (mode === "getItem") {
    storage.getItem = denied;
  } else {
    storage.setItem = denied;
  }
}

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

  it("keeps the default locale when the storage getter throws", () => {
    installDeniedStorage("getter");

    expect(readStoredLocale()).toBeNull();
    expect(resolveInitialLocale()).toBe("en");
    expect(createAppI18n().global.locale.value).toBe("en");
    expect(() => persistLocale("zh-CN")).not.toThrow();
  });

  it("keeps the default locale when a stored read throws", () => {
    installDeniedStorage("getItem");

    expect(readStoredLocale()).toBeNull();
    expect(resolveInitialLocale()).toBe("en");
    expect(createAppI18n().global.locale.value).toBe("en");
    expect(() => persistLocale("zh-CN")).not.toThrow();
  });

  it("keeps a locale change in memory when a storage write throws", () => {
    installDeniedStorage("setItem");
    const i18n = createAppI18n();

    i18n.global.locale.value = "zh-CN";
    expect(() => persistLocale("zh-CN")).not.toThrow();
    expect(i18n.global.locale.value).toBe("zh-CN");
  });

  it("keeps normal storage persistence unchanged", () => {
    persistLocale("zh-CN", window.localStorage);

    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(readStoredLocale(window.localStorage)).toBe("zh-CN");
    expect(persistLocaleOutcome("en", window.localStorage)).toBe(true);
  });

  it("reports the persistence outcome separately from the applied locale", () => {
    installDeniedStorage("setItem");

    expect(persistLocaleOutcome("zh-CN")).toBe(false);
    expect(persistLocaleOutcome("zh-CN", null)).toBe(false);
    expect(persistLocale("zh-CN")).toBeUndefined();
  });
});
