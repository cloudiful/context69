import { LogOut, UserRound } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";

import { Badge } from "@/components/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  SidebarFooter,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";

import { useAuthSession } from "@/hooks/use-auth";
import { logout } from "@/lib/auth-store";

export function UserMenu() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { user } = useAuthSession();

  async function handleLogout() {
    await logout();
    navigate("/login", { replace: true });
  }

  return (
    <SidebarFooter>
      <SidebarMenu>
        <SidebarMenuItem>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <SidebarMenuButton>
                <UserRound aria-hidden="true" />
                <span className="truncate">{user?.display_name ?? t("auth.title")}</span>
              </SidebarMenuButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-56">
              <DropdownMenuLabel className="flex items-center justify-between gap-2">
                <span className="truncate">{user?.display_name ?? ""}</span>
                {user?.is_admin ? <Badge variant="secondary">{t("adminUsers.isAdmin")}</Badge> : null}
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => void handleLogout()}>
                <LogOut aria-hidden="true" />
                <span>{t("auth.logout")}</span>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarFooter>
  );
}
