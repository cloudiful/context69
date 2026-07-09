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
  const locale = storage?.getItem(LOCALE_STORAGE_KEY);
  return normalizeAppLocale(locale);
}

export function resolveInitialLocale(storage: Storage | null | undefined = getStorage()): AppLocale {
  return readStoredLocale(storage) ?? DEFAULT_LOCALE;
}

export function persistLocale(locale: AppLocale, storage: Storage | null | undefined = getStorage()) {
  storage?.setItem(LOCALE_STORAGE_KEY, locale);
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
