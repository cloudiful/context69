import { reactive } from "vue";

import type { AppTheme } from "../types/ui";
import {
  browserTimeZone,
  effectiveTimeZone,
  getTimeZonePreference,
  setTimeZonePreference,
} from "../utils/time-zone";

const THEME_STORAGE_KEY = "context69.theme";

interface UiPreferencesState {
  theme: AppTheme;
  sidebarCollapsed: boolean;
  readonly timeZone: string;
}

const state = reactive<UiPreferencesState>({
  theme: resolveInitialTheme(),
  sidebarCollapsed: false,
  get timeZone() {
    return getTimeZonePreference();
  },
});

let hydrated = false;

function resolveInitialTheme(): AppTheme {
  try {
    const stored = getStorage()?.getItem(THEME_STORAGE_KEY);
    return stored === "light" || stored === "dark" ? stored : "dark";
  } catch {
    return "dark";
  }
}

function persistTheme(theme: AppTheme) {
  try {
    getStorage()?.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // A browser that denies storage keeps the theme for this page only.
  }
}

function applyTheme(theme: AppTheme) {
  if (typeof document === "undefined") {
    return;
  }

  document.documentElement.dataset.theme = theme;
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.style.colorScheme = theme;
}

function hydrate() {
  if (hydrated) {
    return;
  }

  hydrated = true;
  applyTheme(state.theme);
}

function setTheme(theme: AppTheme) {
  state.theme = theme;
  persistTheme(theme);
  applyTheme(theme);
}

function getStorage(): Storage | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    const storage = window.localStorage;
    return storage && typeof storage.getItem === "function" && typeof storage.setItem === "function"
      ? storage
      : null;
  } catch {
    return null;
  }
}

export function useUiPreferences() {
  return {
    state,
    hydrate,
    setTheme,
    browserTimeZone,
    effectiveTimeZone,
    setTimeZone: setTimeZonePreference,
  };
}
