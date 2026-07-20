import { computed } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";

import { authSessionState } from "../services/auth/session";
import { resolveSettingsSectionNav } from "../settings/navigation";
import { resolveGroupSectionNav } from "../workspace/navigation";

export interface AppNavChildItem {
  to: string;
  label: string;
}

export interface AppNavItem {
  to: string;
  activePrefix?: string;
  label: string;
  icon: string;
  children?: AppNavChildItem[];
}

export function useAppNavigation() {
  const route = useRoute();
  const { t } = useI18n();

  const routeName = computed(() => String(route.name ?? ""));
  const groupPath = computed(() => String(route.params.groupPath ?? ""));

  const groupsChildren = computed(() => {
    if (!groupPath.value) {
      return undefined;
    }

    if (routeName.value.startsWith("group-")) {
      return resolveGroupSectionNav(t, groupPath.value);
    }

    return undefined;
  });

  return computed<AppNavItem[]>(() => [
    { to: "/search", label: t("nav.search"), icon: "i-lucide-search" },
    { to: "/processing-queue", label: t("nav.processingQueue"), icon: "i-lucide-list-checks" },
    {
      to: groupPath.value ? `/groups/${encodeURIComponent(groupPath.value)}` : "/groups",
      activePrefix: "/groups",
      label: t("nav.groups"),
      icon: "i-lucide-folder" ,
      children: groupsChildren.value,
    },
    {
      to: "/settings",
      label: t("nav.settings"),
      icon: "i-lucide-settings",
      children: resolveSettingsSectionNav(t, authSessionState.user?.is_admin === true),
    },
  ]);
}
