import { useEffect } from "react";

import { usePreferences } from "@/hooks/use-preferences";
import { hydratePreferences } from "@/lib/preferences-store";

/**
 * Applies the stored theme to the document and keeps the document language in
 * sync with the active locale. No preference is written here; the panel that
 * edits them stores a draft and persists only on its own Save.
 */
export function PreferencesBridge() {
  const { locale } = usePreferences();

  useEffect(() => hydratePreferences(), []);

  useEffect(() => {
    document.documentElement.lang = locale === "zh-CN" ? "zh-CN" : "en";
  }, [locale]);

  return null;
}
