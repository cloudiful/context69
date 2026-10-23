import { Folder, ListChecks, Search, Settings, type LucideIcon } from "lucide-react";

export type NavLabelKey =
  | "nav.search"
  | "nav.groups"
  | "nav.processingQueue"
  | "nav.settings"
  | "document.title"
  | "settings.sections.appearance"
  | "settings.sections.accessTokens"
  | "settings.sections.search"
  | "settings.sections.runtime"
  | "settings.sections.docling"
  | "settings.sections.translation"
  | "settings.sections.adminUsers";

export interface AppNavItem {
  to: string;
  labelKey: NavLabelKey;
  icon: LucideIcon;
}

/**
 * Navigation metadata for the product areas. The feature screens behind
 * these entries are delivered by the later migration phases, so the routes
 * exist and stay addressable while their screens are still incomplete.
 */
export const APP_NAV_ITEMS: AppNavItem[] = [
  { to: "/search", labelKey: "nav.search", icon: Search },
  { to: "/processing-queue", labelKey: "nav.processingQueue", icon: ListChecks },
  { to: "/groups", labelKey: "nav.groups", icon: Folder },
  { to: "/settings", labelKey: "nav.settings", icon: Settings },
];

export const SEGMENT_LABEL_KEYS: Record<string, NavLabelKey> = {
  search: "nav.search",
  groups: "nav.groups",
  "processing-queue": "nav.processingQueue",
  settings: "nav.settings",
  documents: "document.title",
  appearance: "settings.sections.appearance",
  "access-tokens": "settings.sections.accessTokens",
  runtime: "settings.sections.runtime",
  docling: "settings.sections.docling",
  translation: "settings.sections.translation",
  "admin-users": "settings.sections.adminUsers",
};

export function isNavItemActive(pathname: string, to: string): boolean {
  return pathname === to || pathname.startsWith(`${to}/`);
}
