import { computed, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";

import { normalizeAppLocale, persistLocaleOutcome, type AppLocale } from "../i18n/locale";
import type { AppTheme } from "../types/ui";
import { AUTOMATIC_TIME_ZONE, applyTimeZoneSelection, isValidTimeZone } from "../utils/time-zone";
import { useUiPreferences } from "./use-ui-preferences";

export type AppearanceField = "locale" | "theme" | "timeZone";

export interface AppearanceValues {
  locale: AppLocale;
  theme: AppTheme;
  timeZone: string;
}

export type AppearanceSnapshot = AppearanceValues;

export interface AppearanceCommitResult {
  /** Every appearance write reached the browser store. */
  stored: boolean;
  /** Fields this browser could not store; applied in memory regardless. */
  unstoredFields: AppearanceField[];
}

/**
 * The appearance groups' unsaved draft: selects and picker edit it, while the
 * applied global preferences (i18n locale, ui theme, time zone) keep their last
 * saved values until the shared Save commits a snapshot.
 *
 * The saved baseline only advances after a snapshot is completely stored. An
 * unstored commit leaves an unresolved persistence state behind that stays
 * pending independently of the draft: reverting the draft to the old baseline
 * must not hide the already-applied-but-unstored appearance, so Save stays
 * actionable until some later commit is completely stored.
 */
export function useAppearanceDraft() {
  const { locale: i18nLocale } = useI18n({ useScope: "global" });
  const preferences = useUiPreferences();

  const saved = reactive<AppearanceValues>({
    locale: normalizeAppLocale(String(i18nLocale.value)) ?? "en",
    theme: preferences.state.theme,
    timeZone: preferences.state.timeZone,
  });

  const draft = reactive<AppearanceValues>({ ...saved });

  /**
   * True while this page shows an appearance that the browser could not store.
   * Only a completely successful submitted commit clears it; draft edits never
   * do, because that would claim the applied appearance is persisted.
   */
  const unresolvedCommit = ref(false);

  const dirtyFields = computed<AppearanceField[]>(() => {
    const fields: AppearanceField[] = [];
    if (draft.locale !== saved.locale) fields.push("locale");
    if (draft.theme !== saved.theme) fields.push("theme");
    if (draft.timeZone !== saved.timeZone) fields.push("timeZone");
    return fields;
  });

  const hasChanges = computed(() => unresolvedCommit.value || dirtyFields.value.length > 0);

  function updateLocale(value: unknown) {
    const locale = normalizeAppLocale(typeof value === "string" ? value : null);
    if (!locale) {
      return;
    }
    draft.locale = locale;
  }

  function updateTheme(value: unknown) {
    if (value !== "dark" && value !== "light") {
      return;
    }
    draft.theme = value;
  }

  function updateTimeZone(value: unknown) {
    if (typeof value !== "string") {
      return;
    }

    if (value !== AUTOMATIC_TIME_ZONE && !isValidTimeZone(value)) {
      return;
    }

    draft.timeZone = value;
  }

  function captureSnapshot(): AppearanceSnapshot {
    return {
      locale: draft.locale,
      theme: draft.theme,
      timeZone: draft.timeZone,
    };
  }

  /**
   * Apply a captured snapshot in memory and persist it. The whole submitted
   * appearance is applied even when a write fails, so the page never shows a
   * partial mix; the live draft is never mutated, so newer edits survive.
   */
  function commit(snapshot: AppearanceSnapshot): AppearanceCommitResult {
    const unstoredFields: AppearanceField[] = [];

    i18nLocale.value = snapshot.locale;
    if (!persistLocaleOutcome(snapshot.locale)) {
      unstoredFields.push("locale");
    }

    if (!preferences.setTheme(snapshot.theme)) {
      unstoredFields.push("theme");
    }

    if (!applyTimeZoneSelection(snapshot.timeZone).stored) {
      unstoredFields.push("timeZone");
    }

    const stored = unstoredFields.length === 0;
    if (stored) {
      saved.locale = snapshot.locale;
      saved.theme = snapshot.theme;
      saved.timeZone = snapshot.timeZone;
    }

    unresolvedCommit.value = !stored;

    return { stored, unstoredFields };
  }

  return {
    draft,
    saved,
    dirtyFields,
    hasChanges,
    unresolvedCommit,
    updateLocale,
    updateTheme,
    updateTimeZone,
    captureSnapshot,
    commit,
  };
}
