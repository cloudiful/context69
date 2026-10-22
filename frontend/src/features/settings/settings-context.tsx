import { createContext, useContext, type ReactNode } from "react";

import { useAuthSession } from "@/hooks/use-auth";

import { useAdminUsers } from "./use-admin-users";
import { useDoclingRemoteTasks } from "./use-docling-remote-tasks";
import { usePersonalAccessTokens } from "./use-personal-access-tokens";
import { useSettingsPage, type SettingsPageState } from "./use-settings-page";
import { useVectorRebuild } from "./use-vector-rebuild";

interface SettingsContextValue {
  page: SettingsPageState;
  tokens: ReturnType<typeof usePersonalAccessTokens>;
  adminUsers: ReturnType<typeof useAdminUsers>;
  vector: ReturnType<typeof useVectorRebuild>;
  doclingRemote: ReturnType<typeof useDoclingRemoteTasks>;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

/**
 * Owns the settings page state once for every nested section, so the shared
 * Save, the drafts, and the loaded server responses survive section navigation.
 */
export function SettingsProvider({ children }: { children: ReactNode }) {
  const { user } = useAuthSession();
  const page = useSettingsPage();
  const tokens = usePersonalAccessTokens();
  const adminUsers = useAdminUsers(user?.is_admin === true);
  const vector = useVectorRebuild();
  const doclingRemote = useDoclingRemoteTasks();

  return (
    <SettingsContext.Provider value={{ page, tokens, adminUsers, vector, doclingRemote }}>
      {children}
    </SettingsContext.Provider>
  );
}

export function useSettings() {
  const value = useContext(SettingsContext);
  if (!value) {
    throw new Error("Settings section rendered outside SettingsProvider");
  }
  return value;
}
