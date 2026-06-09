import { beforeEach, describe, expect, it } from "vitest";

import { createAppI18n } from ".";
import { installMockStorage } from "../test-utils/storage";
import { LOCALE_STORAGE_KEY, resolveInitialLocale } from "./locale";

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

  it("exposes translated runtime section labels for chinese locale", () => {
    const i18n = createAppI18n("zh-CN");

    expect(i18n.global.t("settings.runtime.providerAccountsTitle")).toBe("Provider 账户");
    expect(i18n.global.t("settings.runtime.sourceConnectionsTitle")).toBe("来源连接");
  });

  it("falls back to english for invalid stored locales", () => {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "fr");

    const i18n = createAppI18n(resolveInitialLocale(window.localStorage));

    expect(i18n.global.locale.value).toBe("en");
  });
});
