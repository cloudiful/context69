import { useSyncExternalStore } from "react";

import { authSessionStore, type AuthSessionSnapshot } from "../lib/auth-store";

export function useAuthSession(): AuthSessionSnapshot {
  return useSyncExternalStore(
    authSessionStore.subscribe,
    authSessionStore.getSnapshot,
    authSessionStore.getSnapshot,
  );
}
