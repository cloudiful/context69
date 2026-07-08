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
      label: t("groups.projectsTitle"),
      to: `/groups/${groupKey}/projects`,
    },
    {
      label: t("groups.membersTitle"),
      to: `/groups/${groupKey}/members`,
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
      label: t("project.tabs.sources"),
      to: `${basePath}/sources`,
    },
    {
      label: t("project.tabs.files"),
      to: `${basePath}/files`,
    },
    {
      label: t("project.tabs.members"),
      to: `${basePath}/members`,
    },
  ];
}
