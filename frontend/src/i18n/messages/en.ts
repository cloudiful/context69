import { common } from "./en/common";
import { app } from "./en/app";
import { nav } from "./en/nav";
import { processingQueue } from "./en/processing-queue";
import { auth } from "./en/auth";
import { language } from "./en/language";
import { theme } from "./en/theme";
import { sidebar } from "./en/sidebar";
import { members } from "./en/members";
import { groups } from "./en/groups";
import { adminUsers } from "./en/admin-users";
import { search } from "./en/search";
import { library } from "./en/library";
import { sources } from "./en/sources";
import { settings } from "./en/settings";
import { document } from "./en/document";
import { metadataIndexes } from "./en/metadata-indexes";
import { notFound } from "./en/not-found";
import { errors } from "./en/errors";

export const en = {
  common,
  app,
  nav,
  processingQueue,
  auth,
  language,
  theme,
  sidebar,
  members,
  groups,
  adminUsers,
  search,
  library,
  sources,
  settings,
  document,
  metadataIndexes,
  notFound,
  errors,
} as const;
