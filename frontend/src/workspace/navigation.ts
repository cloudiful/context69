import type { ComposerTranslation } from "vue-i18n";

export interface WorkspaceSectionNavItem {
  label: string;
  to: string;
}

export function resolveGroupSectionNav(
  t: ComposerTranslation,
  groupPath: string,
): WorkspaceSectionNavItem[] {
  const basePath = `/groups/${encodeURIComponent(groupPath)}`;
  return [
    {
      label: t("groups.membersTitle"),
      to: `${basePath}/members`,
    },
    {
      label: t("nav.settings"),
      to: `${basePath}/settings`,
    },
  ];
}
