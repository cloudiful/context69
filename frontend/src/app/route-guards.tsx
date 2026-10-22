import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, useLocation } from "react-router";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useAuthSession } from "@/hooks/use-auth";
import { ensureSessionReady, restoreSession } from "@/lib/auth-store";

function SessionLoading() {
  const { t } = useTranslation();

  return (
    <div role="status" aria-live="polite" className="grid min-h-screen place-items-center text-sm text-muted-foreground">
      {t("common.loading")}
    </div>
  );
}

export function RequireAuth({ children }: { children: ReactNode }) {
  const { ready, status, user, restoreError } = useAuthSession();
  const location = useLocation();
  const { t } = useTranslation();

  if (!ready) {
    return <SessionLoading />;
  }

  if (restoreError && !user) {
    return (
      <div className="grid min-h-screen place-items-center px-4">
        <div className="grid max-w-md justify-items-center gap-3 text-center">
          <Alert variant="destructive">
            <AlertTitle>{t("auth.sessionUnavailable")}</AlertTitle>
            <AlertDescription>{t("auth.sessionUnavailableMessage")}</AlertDescription>
          </Alert>
          <Button size="sm" onClick={() => void restoreSession()}>
            {t("common.retry")}
          </Button>
        </div>
      </div>
    );
  }

  if (status !== "authenticated" || !user) {
    const search = new URLSearchParams({ redirect: location.pathname + location.search });
    return <Navigate to={`/login?${search.toString()}`} replace />;
  }

  return <>{children}</>;
}

export function RequireGuest({ children }: { children: ReactNode }) {
  const { ready, status, user } = useAuthSession();

  if (!ready) {
    return <SessionLoading />;
  }

  if (status === "authenticated" && user) {
    return <Navigate to="/search" replace />;
  }

  return <>{children}</>;
}

export function RequireAdmin({ children }: { children: ReactNode }) {
  const { user } = useAuthSession();

  if (!user?.is_admin) {
    return <Navigate to="/settings/appearance" replace />;
  }

  return <>{children}</>;
}

/** Restores the cookie session before the guarded tree is evaluated. */
export function AuthBootstrap() {
  void ensureSessionReady();
  return null;
}
