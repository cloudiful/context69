import { useCallback, useMemo, useRef, useState } from "react";

import { usePreferences } from "@/hooks/use-preferences";
import { normalizeAppLocale, type AppLocale } from "@/i18n/locale";
import {
  applyTimeZoneSelection,
  setLocale,
  setTheme,
  type ThemePreference,
} from "@/lib/preferences-store";
import { AUTOMATIC_TIME_ZONE, isValidTimeZone } from "@/lib/time-zone";

export type AppearanceField = "locale" | "theme" | "timeZone";

export interface AppearanceValues {
  locale: AppLocale;
  theme: ThemePreference;
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
 * The appearance groups' unsaved draft: selects and the picker edit it, while
 * the applied preferences keep their last saved values until the shared Save
 * commits a snapshot.
 *
 * The saved baseline only advances after a snapshot is completely stored. An
 * unstored commit leaves an unresolved persistence state behind that stays
 * pending independently of the draft, so reverting the draft to the old
 * baseline cannot hide an already-applied-but-unstored appearance.
 */
export function useAppearanceDraft() {
  const preferences = usePreferences();
  // The baseline comes from the applied preferences at mount; later store
  // changes are this page's own commits, not a new baseline.
  const [saved, setSaved] = useState<AppearanceValues>(() => ({
    locale: preferences.locale,
    theme: preferences.theme,
    timeZone: preferences.timeZone,
  }));
  const [draft, setDraftState] = useState<AppearanceValues>(saved);
  const draftRef = useRef(draft);
  const setDraft = useCallback((next: AppearanceValues) => {
    draftRef.current = next;
    setDraftState(next);
  }, []);
  /**
   * True while the page shows an appearance the browser could not store. Only
   * a completely stored commit clears it; draft edits never do.
   */
  const [unresolvedCommit, setUnresolvedCommit] = useState(false);
  const savedRef = useRef(saved);
  savedRef.current = saved;

  const dirtyFields = useMemo<AppearanceField[]>(() => {
    const fields: AppearanceField[] = [];
    if (draft.locale !== saved.locale) fields.push("locale");
    if (draft.theme !== saved.theme) fields.push("theme");
    if (draft.timeZone !== saved.timeZone) fields.push("timeZone");
    return fields;
  }, [draft, saved]);

  const updateLocale = useCallback((value: string) => {
    const locale = normalizeAppLocale(value);
    if (!locale) {
      return;
    }
    setDraft({ ...draftRef.current, locale });
  }, []);

  const updateTheme = useCallback((value: string) => {
    if (value !== "dark" && value !== "light" && value !== "system") {
      return;
    }
    setDraft({ ...draftRef.current, theme: value });
  }, []);

  const updateTimeZone = useCallback((value: string) => {
    if (value !== AUTOMATIC_TIME_ZONE && !isValidTimeZone(value)) {
      return;
    }
    setDraft({ ...draftRef.current, timeZone: value });
  }, []);

  const captureSnapshot = useCallback((): AppearanceSnapshot => ({ ...draftRef.current }), []);

  /**
   * Apply a captured snapshot in memory and persist it. The whole submitted
   * appearance is applied even when a write fails, so the page never shows a
   * partial mix; the live draft is never mutated, so newer edits survive.
   */
  const commit = useCallback((snapshot: AppearanceSnapshot): AppearanceCommitResult => {
    const unstoredFields: AppearanceField[] = [];

    if (!setLocale(snapshot.locale)) {
      unstoredFields.push("locale");
    }
    if (!setTheme(snapshot.theme)) {
      unstoredFields.push("theme");
    }
    if (!applyTimeZoneSelection(snapshot.timeZone).stored) {
      unstoredFields.push("timeZone");
    }

    const stored = unstoredFields.length === 0;
    if (stored) {
      savedRef.current = { ...snapshot };
      setSaved({ ...snapshot });
    }

    setUnresolvedCommit(!stored);

    return { stored, unstoredFields };
  }, []);

  return {
    draft,
    hasChanges: unresolvedCommit || dirtyFields.length > 0,
    dirtyFields,
    unresolvedCommit,
    updateLocale,
    updateTheme,
    updateTimeZone,
    captureSnapshot,
    commit,
  };
}
