import type { ComposerTranslation } from "vue-i18n";

export interface WorkspaceSectionNavItem {
  label: string;
  to: string;
}

export function resolveGroupSectionNav(
  t: ComposerTranslation,
  groupKey: string,
): WorkspaceSectionNavItem[] {
  return [
    {
      label: t("groups.tabs.overview"),
      to: `/groups/${groupKey}/overview`,
    },
    {
      label: t("groups.membersTitle"),
      to: `/groups/${groupKey}/members`,
    },
    {
      label: t("nav.settings"),
      to: `/groups/${groupKey}/settings`,
    },
  ];
}

export function resolveProjectSectionNav(
  t: ComposerTranslation,
  groupKey: string,
  projectKey: string,
): WorkspaceSectionNavItem[] {
  const basePath = `/groups/${groupKey}/projects/${projectKey}`;
  return [
    {
      label: t("project.tabs.overview"),
      to: `${basePath}/overview`,
    },
    {
      label: t("project.tabs.members"),
      to: `${basePath}/members`,
    },
    {
      label: t("nav.settings"),
      to: `${basePath}/settings`,
    },
  ];
}
