import i18next from "i18next";
import { initReactI18next } from "react-i18next";

import { DEFAULT_LOCALE, resolveInitialLocale, type AppLocale } from "../i18n/locale";
import { en } from "../i18n/messages/en";
import { zhCN } from "../i18n/messages/zh-CN";

/**
 * The shared catalog objects stay the single source for the English and
 * Chinese copy; the React binding only supplies the runtime.
 */
export const i18n = i18next.createInstance();

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    "zh-CN": { translation: zhCN },
  },
  lng: resolveInitialLocale(),
  fallbackLng: DEFAULT_LOCALE,
  supportedLngs: ["en", "zh-CN"],
  interpolation: { escapeValue: false, prefix: "{", suffix: "}" },
});

export function changeAppLocale(locale: AppLocale) {
  if (i18n.language !== locale) {
    void i18n.changeLanguage(locale);
  }
}

export type { AppLocale };
