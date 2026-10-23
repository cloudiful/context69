import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Outlet, useBlocker, useLocation, useNavigate } from "react-router";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAuthSession } from "@/hooks/use-auth";
import { cn } from "@/lib/utils";

import { SettingsProvider, useSettings } from "./settings-context";
import { visibleSettingsSections } from "./settings-nav";

/** Shared settings frame: section navigation, shared Save, and the leave guard. */
export function SettingsLayout() {
  return (
    <SettingsProvider>
      <SettingsLayoutContent />
    </SettingsProvider>
  );
}

function SettingsLayoutContent() {
  const { t } = useTranslation();
  const { user } = useAuthSession();
  const location = useLocation();
  const navigate = useNavigate();
  const { page } = useSettings();
  const [leaveOpen, setLeaveOpen] = useState(false);

  const sections = visibleSettingsSections(user?.is_admin === true);
  const blocker = useBlocker(({ currentLocation, nextLocation }) => (
    page.hasChanges && currentLocation.pathname !== nextLocation.pathname
  ));

  useEffect(() => {
    if (blocker.state === "blocked") {
      setLeaveOpen(true);
    }
  }, [blocker.state]);

  return (
    <section
      data-testid="settings-page-scroll"
      className="grid min-w-0 content-start gap-2 pb-16"
    >
      <div className="overflow-x-auto md:hidden">
        <Tabs
          value={location.pathname}
          onValueChange={(value) => navigate(value)}
          aria-label={t("nav.settings")}
        >
          <TabsList className="w-max">
            {sections.map((section) => (
              <TabsTrigger key={section.key} value={section.path} className="whitespace-nowrap">
                {t(section.labelKey)}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>

      <div className="grid min-w-0 gap-3 md:grid-cols-[11rem_minmax(0,1fr)]">
        <nav aria-label={t("nav.settings")} className="hidden md:block">
          <ul className="grid gap-0.5">
            {sections.map((section) => {
              const active = location.pathname === section.path;
              return (
                <li key={section.key}>
                  <Button
                    asChild
                    variant="ghost"
                    size="sm"
                    className={cn("w-full justify-start", active && "bg-muted font-medium")}
                  >
                    <a
                      href={section.path}
                      aria-current={active ? "page" : undefined}
                      onClick={(event) => {
                        event.preventDefault();
                        navigate(section.path);
                      }}
                    >
                      {t(section.labelKey)}
                    </a>
                  </Button>
                </li>
              );
            })}
          </ul>
        </nav>

        <div className="grid min-w-0 content-start gap-3">
          <div className="flex flex-wrap items-center justify-between gap-1.5">
            <h1 className="text-lg font-semibold">{t("nav.settings")}</h1>
            <div className="flex flex-wrap items-center gap-1.5">
            {page.hasChanges ? (
              <Badge variant="secondary">{t("settings.status.pending")}</Badge>
            ) : null}
              {page.saveMessage ? <Badge variant="outline">{page.saveMessage}</Badge> : null}
            </div>
          </div>

          <Outlet />
        </div>
      </div>

      <div className="fixed inset-x-0 bottom-0 z-30 flex justify-end border-t bg-background/95 px-4 py-2 backdrop-blur">
        <Button
          type="button"
          data-testid="settings-save"
          className="min-w-28"
          disabled={page.saving || !page.hasChanges}
          aria-busy={page.saving}
          onClick={() => void page.saveSettings()}
        >
          {page.saving ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
          {page.saving ? t("common.loading") : t("common.save")}
        </Button>
      </div>

      <ConfirmDialog
        open={leaveOpen}
        title={t("settings.status.pending")}
        message={t("settings.unsavedChanges")}
        acceptLabel={t("settings.leavePage")}
        onAccept={() => {
          setLeaveOpen(false);
          blocker.proceed?.();
        }}
        onCancel={() => {
          setLeaveOpen(false);
          blocker.reset?.();
        }}
      />

      {/*
        The compatibility prompt belongs to the shared Save, not to the runtime
        section: a save can start from any section, and cancelling must hold
        back every group in that save plan.
      */}
      <ConfirmDialog
        open={page.compatibilityOpen}
        title={t("settings.runtime.compatibilityTitle")}
        message={t("settings.runtime.compatibilityConfirm")}
        acceptLabel={t("settings.runtime.compatibilityAccept")}
        onAccept={page.confirmCompatibility}
        onCancel={page.cancelCompatibility}
      />
    </section>
  );
}
