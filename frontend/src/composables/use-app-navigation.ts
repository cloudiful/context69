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
  label: string;
  iconPath: string;
  children?: AppNavChildItem[];
}

const mdiMagnify = "M9.5,3A6.5,6.5 0 0,1 16,9.5C16,11.11 15.41,12.59 14.43,13.73L20.71,20L19.29,21.41L13,15.14C11.87,15.68 10.71,16 9.5,16A6.5,6.5 0 0,1 3,9.5A6.5,6.5 0 0,1 9.5,3M9.5,5A4.5,4.5 0 0,0 5,9.5A4.5,4.5 0 0,0 9.5,14A4.5,4.5 0 0,0 14,9.5A4.5,4.5 0 0,0 9.5,5Z";
const mdiAccountMultipleOutline = "M16,11C18.21,11 20,9.21 20,7C20,4.79 18.21,3 16,3C13.79,3 12,4.79 12,7C12,9.21 13.79,11 16,11M8,11C10.21,11 12,9.21 12,7C12,4.79 10.21,3 8,3C5.79,3 4,4.79 4,7C4,9.21 5.79,11 8,11M8,13C4.67,13 0,14.67 0,18V21H16V18C16,14.67 11.33,13 8,13M8,15C10.67,15 14,16.33 14,18V19H2V18C2,16.33 5.33,15 8,15M16,13C15.76,13 15.5,13 15.22,13.03C16.29,13.83 17,14.94 17,16.31V19H24V18C24,14.67 19.33,13 16,13Z";
const mdiCogOutline = "M19.43,12.98C19.47,12.66 19.5,12.34 19.5,12C19.5,11.66 19.47,11.33 19.43,11L21.54,9.37C21.73,9.22 21.78,8.95 21.66,8.73L19.66,5.27C19.54,5.05 19.27,4.96 19.04,5.05L16.56,6.05C16.04,5.66 15.5,5.32 14.87,5.07L14.5,2.42C14.46,2.18 14.25,2 14,2H10C9.75,2 9.54,2.18 9.5,2.42L9.13,5.07C8.5,5.32 7.96,5.66 7.44,6.05L4.96,5.05C4.73,4.96 4.46,5.05 4.34,5.27L2.34,8.73C2.22,8.95 2.27,9.22 2.46,9.37L4.57,11C4.53,11.33 4.5,11.66 4.5,12C4.5,12.34 4.53,12.66 4.57,12.98L2.46,14.63C2.27,14.78 2.22,15.05 2.34,15.27L4.34,18.73C4.46,18.95 4.73,19.04 4.96,18.95L7.44,17.95C7.96,18.34 8.5,18.68 9.13,18.93L9.5,21.58C9.54,21.82 9.75,22 10,22H14C14.25,22 14.46,21.82 14.5,21.58L14.87,18.93C15.5,18.68 16.04,18.34 16.56,17.95L19.04,18.95C19.27,19.04 19.54,18.95 19.66,18.73L21.66,15.27C21.78,15.05 21.73,14.78 21.54,14.63L19.43,12.98M12,15.5A3.5,3.5 0 0,1 8.5,12A3.5,3.5 0 0,1 12,8.5A3.5,3.5 0 0,1 15.5,12A3.5,3.5 0 0,1 12,15.5Z";

export function useAppNavigation() {
  const route = useRoute();
  const { t } = useI18n();

  const routeName = computed(() => String(route.name ?? ""));
  const groupPath = computed(() => String(route.params.groupPath ?? ""));

  const groupsChildren = computed(() => {
    if (!groupPath.value) {
      return undefined;
    }

    if (routeName.value.startsWith("group-") || routeName.value === "group-detail") {
      return resolveGroupSectionNav(t, groupPath.value);
    }

    return undefined;
  });

  return computed<AppNavItem[]>(() => [
    { to: "/search", label: t("nav.search"), iconPath: mdiMagnify },
    {
      to: "/groups",
      label: t("nav.groups"),
      iconPath: mdiAccountMultipleOutline,
      children: groupsChildren.value,
    },
    {
      to: "/settings",
      label: t("nav.settings"),
      iconPath: mdiCogOutline,
      children: resolveSettingsSectionNav(t, authSessionState.user?.is_admin === true),
    },
  ]);
}
