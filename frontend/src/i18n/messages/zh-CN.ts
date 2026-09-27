import { common } from "./zh-CN/common";
import { app } from "./zh-CN/app";
import { nav } from "./zh-CN/nav";
import { processingQueue } from "./zh-CN/processing-queue";
import { auth } from "./zh-CN/auth";
import { language } from "./zh-CN/language";
import { theme } from "./zh-CN/theme";
import { sidebar } from "./zh-CN/sidebar";
import { members } from "./zh-CN/members";
import { groups } from "./zh-CN/groups";
import { adminUsers } from "./zh-CN/admin-users";
import { search } from "./zh-CN/search";
import { library } from "./zh-CN/library";
import { sources } from "./zh-CN/sources";
import { settings } from "./zh-CN/settings";
import { document } from "./zh-CN/document";
import { metadataIndexes } from "./zh-CN/metadata-indexes";
import { notFound } from "./zh-CN/not-found";
import { errors } from "./zh-CN/errors";

export const zhCN = {
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
