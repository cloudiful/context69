import { beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "../test-utils/storage";

const THEME_STORAGE_KEY = "context69.theme";

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

async function importPreferences() {
  vi.resetModules();
  return import("./use-ui-preferences");
}

describe("ui preferences under browser storage denial", () => {
  beforeEach(() => {
    installMockStorage();
    window.localStorage.clear();
  });

  it("restores persisted theme and locale-compatible defaults normally", async () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "light");
    const { useUiPreferences } = await importPreferences();
    const preferences = useUiPreferences();

    expect(preferences.state.theme).toBe("light");
    expect(preferences.state.sidebarCollapsed).toBe(false);
  });

  it("keeps the default theme and still applies changes when the storage getter throws", async () => {
    installDeniedStorage("getter");
    const { useUiPreferences } = await importPreferences();
    const preferences = useUiPreferences();

    expect(preferences.state.theme).toBe("dark");
    expect(() => preferences.setTheme("light")).not.toThrow();
    expect(preferences.state.theme).toBe("light");

    preferences.hydrate();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("keeps the default theme when a stored read throws", async () => {
    installDeniedStorage("getItem");
    const { useUiPreferences } = await importPreferences();
    const preferences = useUiPreferences();

    expect(preferences.state.theme).toBe("dark");
    expect(() => preferences.setTheme("light")).not.toThrow();
    expect(preferences.state.theme).toBe("light");
  });

  it("keeps theme changes in memory when a storage write throws", async () => {
    installDeniedStorage("setItem");
    const { useUiPreferences } = await importPreferences();
    const preferences = useUiPreferences();

    expect(preferences.state.theme).toBe("dark");
    expect(preferences.setTheme("light")).toBe(false);
    expect(preferences.state.theme).toBe("light");
    expect(() => preferences.hydrate()).not.toThrow();
    expect(preferences.hydrate).toBeTypeOf("function");
  });

  it("reports a stored theme write as successful", async () => {
    const { useUiPreferences } = await importPreferences();
    const preferences = useUiPreferences();

    expect(preferences.setTheme("light")).toBe(true);
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
  });

  it("hydrates once and leaves the selected theme untouched", async () => {
    const { useUiPreferences } = await importPreferences();
    const preferences = useUiPreferences();

    preferences.setTheme("light");
    preferences.hydrate();
    expect(document.documentElement.dataset.theme).toBe("light");

    preferences.hydrate();
    expect(preferences.state.theme).toBe("light");
  });
});
