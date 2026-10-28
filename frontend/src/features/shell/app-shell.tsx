import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Outlet } from "react-router";

import { Separator } from "@/components/ui/separator";
import { SidebarInset, SidebarProvider, SidebarTrigger, useSidebar } from "@/components/ui/sidebar";

import { AppBreadcrumbs } from "./app-breadcrumbs";
import { AppSidebar } from "./app-sidebar";

const SHELL_SIDEBAR_STYLE = { "--sidebar-width": "13rem" } as CSSProperties;

export function AppShell() {
  return (
    <SidebarProvider style={SHELL_SIDEBAR_STYLE}>
      <AppSidebar />
      <SidebarInset className="min-w-0 overflow-hidden">
        <ShellHeader />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-x-hidden px-3 py-3">
          <Outlet />
        </main>
      </SidebarInset>
    </SidebarProvider>
  );
}

function ShellHeader() {
  const { t } = useTranslation();
  const { isMobile, openMobile, state } = useSidebar();
  const expands = isMobile ? !openMobile : state === "collapsed";

  return (
    <header className="flex min-w-0 items-center gap-2 border-b bg-card px-3 py-2">
      <SidebarTrigger aria-label={expands ? t("sidebar.expand") : t("sidebar.collapse")} />
      <Separator orientation="vertical" className="h-5" />
      <AppBreadcrumbs />
    </header>
  );
}
