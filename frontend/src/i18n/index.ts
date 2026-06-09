import { createI18n } from "vue-i18n";

import { messages } from "./messages";
import { DEFAULT_LOCALE, resolveInitialLocale, type AppLocale } from "./locale";

export function createAppI18n(locale: AppLocale = resolveInitialLocale()) {
  return createI18n({
    legacy: false,
    globalInjection: true,
    locale,
    fallbackLocale: DEFAULT_LOCALE,
    messages,
  });
}

export const i18n = createAppI18n();
