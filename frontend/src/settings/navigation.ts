import type { ComposerTranslation } from "vue-i18n";

export type SettingsSectionKey =
  | "appearance"
  | "access-tokens"
  | "search"
  | "runtime"
  | "docling"
  | "admin-users";

export interface SettingsSectionNavItem {
  key: SettingsSectionKey;
  label: string;
  to: string;
}

export function resolveSettingsSectionNav(
  t: ComposerTranslation,
  isAdmin: boolean,
): SettingsSectionNavItem[] {
  const items: SettingsSectionNavItem[] = [
    {
      key: "appearance",
      label: t("settings.sections.appearance"),
      to: "/settings/appearance",
    },
    {
      key: "access-tokens",
      label: t("settings.sections.accessTokens"),
      to: "/settings/access-tokens",
    },
    {
      key: "search",
      label: t("settings.sections.search"),
      to: "/settings/search",
    },
    {
      key: "runtime",
      label: t("settings.sections.runtime"),
      to: "/settings/runtime",
    },
    {
      key: "docling",
      label: t("settings.sections.docling"),
      to: "/settings/docling",
    },
  ];

  if (isAdmin) {
    items.push({
      key: "admin-users",
      label: t("settings.sections.adminUsers"),
      to: "/settings/admin-users",
    });
  }

  return items;
}
