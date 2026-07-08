import { inject, type InjectionKey } from "vue";

import type { SettingsPageState } from "./use-settings-page";

export const settingsPageStateKey: InjectionKey<SettingsPageState> = Symbol("settings-page-state");

export function useSettingsPageContext(): SettingsPageState {
  const state = inject(settingsPageStateKey);
  if (!state) {
    throw new Error("settings page state is not available");
  }
  return state;
}
