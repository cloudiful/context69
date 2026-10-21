export const LOCALE_STORAGE_KEY = "context69.locale";
export const DEFAULT_LOCALE = "en";
export const SUPPORTED_LOCALES = ["en", "zh-CN"] as const;

export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

export function normalizeAppLocale(value: string | null | undefined): AppLocale | null {
  if (!value) {
    return null;
  }

  const normalized = value.trim().toLowerCase().replace(/_/g, "-");
  if (normalized === "zh" || normalized === "zh-cn" || normalized === "zh-hans" || normalized.startsWith("zh-")) {
    return "zh-CN";
  }

  if (normalized === "en" || normalized.startsWith("en-")) {
    return "en";
  }

  return null;
}

export function isAppLocale(value: string | null | undefined): value is AppLocale {
  return normalizeAppLocale(value) !== null;
}

export function readStoredLocale(storage: Storage | null | undefined = getStorage()): AppLocale | null {
  if (!storage) {
    return null;
  }

  try {
    return normalizeAppLocale(storage.getItem(LOCALE_STORAGE_KEY));
  } catch {
    return null;
  }
}

export function resolveInitialLocale(storage: Storage | null | undefined = getStorage()): AppLocale {
  return readStoredLocale(storage) ?? DEFAULT_LOCALE;
}

export function persistLocale(locale: AppLocale, storage: Storage | null | undefined = getStorage()) {
  persistLocaleOutcome(locale, storage);
}

/**
 * Persist the language choice and report whether the browser actually stored
 * it, so an aggregate Save can distinguish a stored preference from an
 * in-memory-only one.
 */
export function persistLocaleOutcome(
  locale: AppLocale,
  storage: Storage | null | undefined = getStorage(),
): boolean {
  if (!storage) {
    return false;
  }

  try {
    storage.setItem(LOCALE_STORAGE_KEY, locale);
    return true;
  } catch {
    // A browser that denies storage keeps the language for this page only.
    return false;
  }
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
