import type { AppLocale } from "../i18n/locale";
import { createAppI18n } from "../i18n";

export function createTestI18n(locale: AppLocale = "en") {
  return createAppI18n(locale);
}
