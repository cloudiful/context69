import {
  DEFAULT_LOCALE,
  LOCALE_STORAGE_KEY,
  normalizeAppLocale,
  persistLocaleOutcome,
  readStoredLocale,
  type AppLocale,
} from "../i18n/locale";
import { changeAppLocale } from "./i18n";
import {
  AUTOMATIC_TIME_ZONE,
  browserTimeZone,
  getStorage,
  isValidTimeZone,
  resolveInitialTimeZone,
  TIME_ZONE_STORAGE_KEY,
  type TimeZonePreference,
} from "./time-zone";

export const THEME_STORAGE_KEY = "context69.theme";

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export interface PreferencesSnapshot {
  theme: ThemePreference;
  resolvedTheme: ResolvedTheme;
  locale: AppLocale;
  timeZone: TimeZonePreference;
  effectiveTimeZone: string;
}

export interface TimeZoneSelectionResult {
  /** The selection passed validation and is now the applied preference. */
  applied: boolean;
  /** The browser stored the selection; false for a denied or unavailable store. */
  stored: boolean;
}

function readStoredTheme(storage: Storage | null | undefined = getStorage()): ThemePreference | null {
  if (!storage) {
    return null;
  }

  try {
    const stored = storage.getItem(THEME_STORAGE_KEY);
    if (stored === "light" || stored === "dark" || stored === "system") {
      return stored;
    }
  } catch {
    return null;
  }

  return null;
}

export function resolveInitialTheme(storage: Storage | null | undefined = getStorage()): ThemePreference {
  return readStoredTheme(storage) ?? "system";
}

function prefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }

  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false;
  }
}

export function resolveTheme(theme: ThemePreference): ResolvedTheme {
  if (theme === "system") {
    return prefersDark() ? "dark" : "light";
  }

  return theme;
}

function applyThemeToDocument(resolved: ResolvedTheme) {
  if (typeof document === "undefined") {
    return;
  }

  document.documentElement.dataset.theme = resolved;
  document.documentElement.classList.toggle("dark", resolved === "dark");
  document.documentElement.style.colorScheme = resolved;
}

let theme: ThemePreference = resolveInitialTheme();
let locale: AppLocale = readStoredLocale() ?? DEFAULT_LOCALE;
let timeZone: TimeZonePreference = resolveInitialTimeZone();

let snapshot: PreferencesSnapshot = buildSnapshot();
const listeners = new Set<() => void>();

function buildSnapshot(): PreferencesSnapshot {
  return {
    theme,
    resolvedTheme: resolveTheme(theme),
    locale,
    timeZone,
    effectiveTimeZone: timeZone === AUTOMATIC_TIME_ZONE ? browserTimeZone : timeZone,
  };
}

function emit() {
  snapshot = buildSnapshot();
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): PreferencesSnapshot {
  return snapshot;
}

function persistTheme(value: ThemePreference): boolean {
  const storage = getStorage();
  if (!storage) {
    return false;
  }

  try {
    storage.setItem(THEME_STORAGE_KEY, value);
    return true;
  } catch {
    // A browser that denies storage keeps the theme for this page only.
    return false;
  }
}

/** Apply a theme immediately and report whether the browser stored it. */
export function setTheme(value: ThemePreference): boolean {
  theme = value;
  applyThemeToDocument(resolveTheme(value));
  const stored = persistTheme(value);
  emit();
  return stored;
}

/** Apply a language immediately and report whether the browser stored it. */
export function setLocale(value: AppLocale): boolean {
  locale = normalizeAppLocale(value) ?? DEFAULT_LOCALE;
  const stored = persistLocaleOutcome(locale);
  changeAppLocale(locale);
  emit();
  return stored;
}

/**
 * Apply a validated time zone selection in memory and persist it, reporting
 * both parts separately.
 */
export function applyTimeZoneSelection(value: string): TimeZoneSelectionResult {
  if (value !== AUTOMATIC_TIME_ZONE && !isValidTimeZone(value)) {
    return { applied: false, stored: false };
  }

  timeZone = value;
  const stored = persistTimeZone(value);
  emit();
  return { applied: true, stored };
}

export function setTimeZone(value: string): boolean {
  return applyTimeZoneSelection(value).applied;
}

function persistTimeZone(preference: string): boolean {
  const storage = getStorage();
  if (!storage) {
    return false;
  }

  try {
    storage.setItem(TIME_ZONE_STORAGE_KEY, preference);
    return true;
  } catch {
    // A browser that denies storage keeps the selection for this page only.
    return false;
  }
}

/** Apply the stored appearance choices to the document and keep them in sync. */
export function hydratePreferences() {
  applyThemeToDocument(resolveTheme(theme));

  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return () => undefined;
  }

  let query: MediaQueryList;
  try {
    query = window.matchMedia("(prefers-color-scheme: dark)");
  } catch {
    return () => undefined;
  }

  const onSystemThemeChange = () => {
    if (theme !== "system") {
      return;
    }

    applyThemeToDocument(resolveTheme(theme));
    emit();
  };

  query.addEventListener?.("change", onSystemThemeChange);
  return () => query.removeEventListener?.("change", onSystemThemeChange);
}

export const preferencesStore = {
  subscribe,
  getSnapshot,
};

/** Effective display zone for formatting outside React (defaults to the browser zone). */
export function getEffectiveTimeZone(): string {
  return snapshot.effectiveTimeZone;
}
