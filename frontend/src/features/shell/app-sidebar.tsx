import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Link, useLocation } from "react-router";

import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";

import { APP_NAV_ITEMS, isNavItemActive } from "./app-nav";
import { UserMenu } from "./user-menu";

export function AppSidebar() {
  const { t } = useTranslation();
  const location = useLocation();
  const { setOpenMobile } = useSidebar();

  useEffect(() => {
    setOpenMobile(false);
  }, [location.pathname, setOpenMobile]);

  return (
    <Sidebar collapsible="icon">
      <SidebarContent>
        <SidebarGroup>
          <nav aria-label={t("app.title")}>
            <SidebarMenu>
              {APP_NAV_ITEMS.map((item) => {
                const active = isNavItemActive(location.pathname, item.to);

                return (
                  <SidebarMenuItem key={item.to}>
                    <SidebarMenuButton asChild isActive={active} tooltip={t(item.labelKey)}>
                      <Link to={item.to} aria-current={active ? "page" : undefined}>
                        <item.icon aria-hidden="true" />
                        <span>{t(item.labelKey)}</span>
                      </Link>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                );
              })}
            </SidebarMenu>
          </nav>
        </SidebarGroup>
      </SidebarContent>
      <UserMenu />
    </Sidebar>
  );
}
