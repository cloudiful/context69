import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";

import { AppProviders } from "./app/providers";
import { appRoutes } from "./app/router";
import { AppToaster } from "./app/toaster";
import { PreferencesBridge } from "./features/preferences/preferences-bridge";
import { ensureSessionReady, setAuthNavigator } from "./lib/auth-store";
import "./style.css";

const router = createBrowserRouter(appRoutes);

setAuthNavigator((to) => {
  const target = to ?? `${window.location.pathname}${window.location.search}`;
  const search = new URLSearchParams();

  if (target && target !== "/login") {
    search.set("redirect", target);
  }

  search.set("reason", "expired");
  void router.navigate(`/login?${search.toString()}`, { replace: true });
});

async function bootstrap() {
  await ensureSessionReady();

  const container = document.getElementById("app");
  if (!container) {
    throw new Error("Application root is missing");
  }

  createRoot(container).render(
    <StrictMode>
      <AppProviders>
        <PreferencesBridge />
        <RouterProvider router={router} />
        <AppToaster />
      </AppProviders>
    </StrictMode>,
  );
}

void bootstrap();
