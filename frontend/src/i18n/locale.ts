export const LOCALE_STORAGE_KEY = "context69.locale";
export const DEFAULT_LOCALE = "en";
export const SUPPORTED_LOCALES = ["en", "zh-CN"] as const;

export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

export function isAppLocale(value: string | null | undefined): value is AppLocale {
  return Boolean(value && SUPPORTED_LOCALES.includes(value as AppLocale));
}

export function readStoredLocale(storage: Storage | null | undefined = getStorage()): AppLocale | null {
  const locale = storage?.getItem(LOCALE_STORAGE_KEY);
  return isAppLocale(locale) ? locale : null;
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
