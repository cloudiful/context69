import { reactive } from "vue";

import type { AppTheme } from "../types/ui";

const THEME_STORAGE_KEY = "context69.theme";
const SIDEBAR_STORAGE_KEY = "context69.sidebar-collapsed";

interface UiPreferencesState {
  theme: AppTheme;
  sidebarCollapsed: boolean;
}

const state = reactive<UiPreferencesState>({
  theme: resolveInitialTheme(),
  sidebarCollapsed: readStoredSidebarCollapsed(),
});

let hydrated = false;

function resolveInitialTheme(): AppTheme {
  const stored = getStorage()?.getItem(THEME_STORAGE_KEY);
  return stored === "light" || stored === "dark" ? stored : "dark";
}

function readStoredSidebarCollapsed(): boolean {
  return getStorage()?.getItem(SIDEBAR_STORAGE_KEY) === "true";
}

function persistTheme(theme: AppTheme) {
  getStorage()?.setItem(THEME_STORAGE_KEY, theme);
}

function persistSidebarCollapsed(sidebarCollapsed: boolean) {
  getStorage()?.setItem(SIDEBAR_STORAGE_KEY, String(sidebarCollapsed));
}

function applyTheme(theme: AppTheme) {
  if (typeof document === "undefined") {
    return;
  }

  document.documentElement.dataset.theme = theme;
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

function toggleTheme() {
  setTheme(state.theme === "dark" ? "light" : "dark");
}

function toggleSidebar() {
  state.sidebarCollapsed = !state.sidebarCollapsed;
  persistSidebarCollapsed(state.sidebarCollapsed);
}

function getStorage(): Storage | null {
  if (typeof window === "undefined") {
    return null;
  }

  const storage = window.localStorage;
  return storage && typeof storage.getItem === "function" && typeof storage.setItem === "function"
    ? storage
    : null;
}

export function useUiPreferences() {
  return {
    state,
    hydrate,
    setTheme,
    toggleTheme,
    toggleSidebar,
  };
}
