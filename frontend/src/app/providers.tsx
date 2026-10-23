import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { I18nextProvider } from "react-i18next";
import { TooltipProvider } from "@/components/ui/tooltip";

import { authSessionStore, type AuthSessionSnapshot } from "../lib/auth-store";
import { i18n } from "../lib/i18n";

function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        staleTime: 30_000,
      },
      mutations: {
        retry: false,
      },
    },
  });
}

/**
 * The identity a query cache belongs to. A session with no user is the guest
 * boundary, so signing in, signing out and switching accounts each start from a
 * cache that never held another account's responses. An intermediate guest
 * session therefore separates two sessions of the same account as well.
 */
function identitySignature(snapshot: AuthSessionSnapshot): string {
  return snapshot.user ? `user:${snapshot.user.user_id}` : "guest";
}

/**
 * Authentication is the lifetime of the query cache: each identity gets its own
 * client, and the previous client is retired before its replacement renders, so
 * a response that arrives late can only ever reach the cache of the identity
 * that requested it. The provider subtree is keyed by the same identity, so
 * local drafts and selections cannot survive an account switch either.
 */
export function AppProviders({ children }: { children: ReactNode }) {
  const session = useSyncExternalStore(
    authSessionStore.subscribe,
    authSessionStore.getSnapshot,
    authSessionStore.getSnapshot,
  );
  const signature = identitySignature(session);
  const [scope, setScope] = useState(() => ({ signature, client: createQueryClient() }));

  if (scope.signature !== signature) {
    setScope({ signature, client: createQueryClient() });
  }

  const activeClient = useRef(scope.client);
  useEffect(() => {
    const retired = activeClient.current;
    activeClient.current = scope.client;
    if (retired === scope.client) {
      return;
    }
    void retired.cancelQueries();
    retired.getMutationCache().clear();
    retired.clear();
  }, [scope.client]);

  return (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider key={signature} client={scope.client}>
        <TooltipProvider delayDuration={200}>{children}</TooltipProvider>
      </QueryClientProvider>
    </I18nextProvider>
  );
}
