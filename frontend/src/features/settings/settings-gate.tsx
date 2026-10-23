import { Loader2 } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

import { useSettings } from "./settings-context";
import { errorMessage } from "./settings-error";

/**
 * Holds a section's fields until the shared settings load has produced drafts,
 * so no section renders default values as if they were the stored settings.
 */
export function SettingsGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const { page } = useSettings();

  if (page.loadError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>{t("settings.loadFailed")}</AlertTitle>
        <AlertDescription className="grid gap-2">
          <span>{errorMessage(page.loadError, t("settings.loadFailed"))}</span>
          <Button size="sm" variant="outline" className="w-fit" onClick={page.reload}>
            {t("common.retry")}
          </Button>
        </AlertDescription>
      </Alert>
    );
  }

  if (page.loading) {
    return (
      <div role="status" className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
        <Loader2 aria-hidden="true" className="size-4 animate-spin" />
        {t("settings.loadingMessage")}
      </div>
    );
  }

  return <>{children}</>;
}
