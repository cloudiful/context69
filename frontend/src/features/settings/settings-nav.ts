export type SettingsSectionKey =
  | "appearance"
  | "access-tokens"
  | "search"
  | "embedding"
  | "chunking"
  | "file-library"
  | "scheduler"
  | "vector-index"
  | "docling"
  | "translation"
  | "admin-users";

export interface SettingsSectionItem {
  key: SettingsSectionKey;
  path: string;
  labelKey: string;
  adminOnly?: boolean;
}

/** Settings sections in navigation order; the admin-only ones appear for admins. */
export const SETTINGS_SECTIONS: SettingsSectionItem[] = [
  { key: "appearance", path: "/settings/appearance", labelKey: "settings.sections.appearance" },
  { key: "access-tokens", path: "/settings/access-tokens", labelKey: "settings.sections.accessTokens" },
  { key: "search", path: "/settings/search", labelKey: "settings.sections.search" },
  { key: "embedding", path: "/settings/embedding", labelKey: "settings.sections.embedding" },
  { key: "chunking", path: "/settings/chunking", labelKey: "settings.sections.chunking" },
  { key: "file-library", path: "/settings/file-library", labelKey: "settings.sections.fileLibrary" },
  { key: "scheduler", path: "/settings/scheduler", labelKey: "settings.sections.scheduler" },
  { key: "vector-index", path: "/settings/vector-index", labelKey: "settings.sections.vectorIndex" },
  { key: "docling", path: "/settings/docling", labelKey: "settings.sections.docling" },
  { key: "translation", path: "/settings/translation", labelKey: "settings.sections.translation", adminOnly: true },
  { key: "admin-users", path: "/settings/admin-users", labelKey: "settings.sections.adminUsers", adminOnly: true },
];

export function visibleSettingsSections(isAdmin: boolean): SettingsSectionItem[] {
  return SETTINGS_SECTIONS.filter((section) => !section.adminOnly || isAdmin);
}
