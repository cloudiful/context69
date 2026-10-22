import { Navigate, type RouteObject } from "react-router";

import { LoginPage } from "@/features/auth/login-page";
import { AppShell } from "@/features/shell/app-shell";
import { NotFoundPage } from "@/features/shell/not-found-page";
import { AccessTokensSection } from "@/features/settings/sections/access-tokens-section";
import { AdminUsersSection } from "@/features/settings/sections/admin-users-section";
import { AppearanceSection } from "@/features/settings/sections/appearance-section";
import { DoclingSection } from "@/features/settings/sections/docling-section";
import { RuntimeSection } from "@/features/settings/sections/runtime-section";
import { SearchSection } from "@/features/settings/sections/search-section";
import { TranslationSection } from "@/features/settings/sections/translation-section";
import { SettingsLayout } from "@/features/settings/settings-layout";
import { SearchPage } from "@/features/search/search-page";
import { GroupLayout } from "@/features/groups/group-layout";
import { GroupMembersPage } from "@/features/groups/group-members-page";
import { GroupOverviewPage } from "@/features/groups/group-overview-page";
import { GroupSettingsPage } from "@/features/groups/group-settings-page";
import { GroupsListPage } from "@/features/groups/groups-list-page";
import { DocumentPage } from "@/features/documents/document-page";
import { ProcessingQueuePage } from "@/features/processing-queue/queue-page";
import { RequireAdmin, RequireAuth, RequireGuest } from "./route-guards";

export const appRoutes: RouteObject[] = [
  {
    path: "/login",
    element: (
      <RequireGuest>
        <LoginPage />
      </RequireGuest>
    ),
  },
  {
    path: "/",
    element: (
      <RequireAuth>
        <AppShell />
      </RequireAuth>
    ),
    children: [
      { index: true, element: <Navigate to="/search" replace /> },
      { path: "search", element: <SearchPage /> },
      { path: "processing-queue", element: <ProcessingQueuePage /> },
      { path: "groups", element: <GroupsListPage /> },
      {
        path: "groups/:groupPath",
        element: <GroupLayout />,
        children: [
          { index: true, element: <GroupOverviewPage /> },
          { path: "overview", element: <Navigate to=".." relative="path" replace /> },
          { path: "members", element: <GroupMembersPage /> },
          { path: "settings", element: <GroupSettingsPage /> },
        ],
      },
      { path: "sources", element: <Navigate to="/groups" replace /> },
      { path: "library", element: <Navigate to="/groups" replace /> },
      { path: "documents/:id", element: <DocumentPage /> },
      {
        path: "settings",
        element: <SettingsLayout />,
        children: [
          { index: true, element: <Navigate to="appearance" replace /> },
          { path: "appearance", element: <AppearanceSection /> },
          { path: "access-tokens", element: <AccessTokensSection /> },
          { path: "search", element: <SearchSection /> },
          { path: "runtime", element: <RuntimeSection /> },
          { path: "docling", element: <DoclingSection /> },
          {
            path: "translation",
            element: (
              <RequireAdmin>
                <TranslationSection />
              </RequireAdmin>
            ),
          },
          {
            path: "admin-users",
            element: (
              <RequireAdmin>
                <AdminUsersSection />
              </RequireAdmin>
            ),
          },
        ],
      },
    ],
  },
  { path: "*", element: <NotFoundPage /> },
];
