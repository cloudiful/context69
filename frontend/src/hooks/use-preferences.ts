import { useSyncExternalStore } from "react";

import { preferencesStore, type PreferencesSnapshot } from "../lib/preferences-store";

export function usePreferences(): PreferencesSnapshot {
  return useSyncExternalStore(
    preferencesStore.subscribe,
    preferencesStore.getSnapshot,
    preferencesStore.getSnapshot,
  );
}

export function useResolvedTheme() {
  return usePreferences().resolvedTheme;
}
