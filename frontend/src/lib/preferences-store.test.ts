import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "../test-utils/storage";

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

function setSystemDark(matches: boolean) {
  window.matchMedia = ((query: string) => ({
    matches: query.includes("dark") ? matches : false,
    media: query,
    onchange: null,
    addListener: () => undefined,
    removeListener: () => undefined,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

describe("neutral preference stores", () => {
  beforeEach(() => {
    vi.resetModules();
    installMockStorage();
    setSystemDark(false);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("defaults to the system theme and the english locale", async () => {
    setSystemDark(true);
    const store = await import("./preferences-store");

    const snapshot = store.preferencesStore.getSnapshot();

    expect(snapshot.theme).toBe("system");
    expect(snapshot.resolvedTheme).toBe("dark");
    expect(snapshot.locale).toBe("en");
    expect(snapshot.timeZone).toBe("auto");
  });

  it("keeps an explicitly stored light or dark theme", async () => {
    window.localStorage.setItem("context69.theme", "light");
    const store = await import("./preferences-store");

    expect(store.preferencesStore.getSnapshot().theme).toBe("light");
    expect(store.preferencesStore.getSnapshot().resolvedTheme).toBe("light");
  });

  it("ignores an invalid stored theme instead of migrating it", async () => {
    window.localStorage.setItem("context69.theme", "sepia");
    const store = await import("./preferences-store");

    expect(store.preferencesStore.getSnapshot().theme).toBe("system");
    expect(window.localStorage.getItem("context69.theme")).toBe("sepia");
  });

  it("applies and persists a theme change and notifies subscribers", async () => {
    const store = await import("./preferences-store");
    const listener = vi.fn();
    store.preferencesStore.subscribe(listener);

    expect(store.setTheme("dark")).toBe(true);

    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.preferencesStore.getSnapshot().resolvedTheme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(window.localStorage.getItem("context69.theme")).toBe("dark");
  });

  it("keeps a theme change in memory when a storage write is denied", async () => {
    const store = await import("./preferences-store");
    installDeniedStorage("setItem");

    expect(store.setTheme("dark")).toBe(false);
    expect(store.preferencesStore.getSnapshot().theme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("starts from the defaults when the storage getter throws", async () => {
    installDeniedStorage("getter");
    const store = await import("./preferences-store");

    expect(store.preferencesStore.getSnapshot().theme).toBe("system");
    expect(store.preferencesStore.getSnapshot().locale).toBe("en");
    expect(() => store.setTheme("light")).not.toThrow();
    expect(store.setTheme("light")).toBe(false);
  });

  it("normalizes and persists a locale change through the shared catalog", async () => {
    const store = await import("./preferences-store");

    expect(store.setLocale("zh-CN")).toBe(true);

    expect(store.preferencesStore.getSnapshot().locale).toBe("zh-CN");
    expect(window.localStorage.getItem("context69.locale")).toBe("zh-CN");
    expect((await import("./i18n")).i18n.language).toBe("zh-CN");
  });

  it("applies a valid time zone and reports a denied store separately", async () => {
    const store = await import("./preferences-store");
    const applied = store.applyTimeZoneSelection("Asia/Tokyo");

    expect(applied).toEqual({ applied: true, stored: true });
    expect(store.preferencesStore.getSnapshot().timeZone).toBe("Asia/Tokyo");
    expect(store.preferencesStore.getSnapshot().effectiveTimeZone).toBe("Asia/Tokyo");
    expect(window.localStorage.getItem("context69.timeZone")).toBe("Asia/Tokyo");

    installDeniedStorage("setItem");
    expect(store.applyTimeZoneSelection("UTC")).toEqual({ applied: true, stored: false });
    expect(store.preferencesStore.getSnapshot().timeZone).toBe("UTC");
  });

  it("rejects an invalid time zone without changing the applied value", async () => {
    const store = await import("./preferences-store");

    expect(store.applyTimeZoneSelection("Mars/Olympus")).toEqual({ applied: false, stored: false });
    expect(store.preferencesStore.getSnapshot().timeZone).toBe("auto");
  });

  it("falls back to the automatic zone for an unknown stored value", async () => {
    window.localStorage.setItem("context69.timeZone", "Mars/Olympus");
    const store = await import("./preferences-store");

    expect(store.preferencesStore.getSnapshot().timeZone).toBe("auto");
  });
});
