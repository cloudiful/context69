import { render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "./providers";
import { appRoutes } from "./router";
import { clearSession, restoreSession } from "../lib/auth-store";

const user = {
  user_id: 7,
  login_name: "planner",
  display_name: "Planner",
  is_admin: false,
  personal_group_path: "personal-planner",
};

async function authenticate(overrides: Partial<typeof user> = {}) {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(
    JSON.stringify({ user: { ...user, ...overrides } }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  )));
  await restoreSession();
}

function renderRoute(path: string) {
  const router = createMemoryRouter(appRoutes, { initialEntries: [path] });
  render(
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>,
  );
  return router;
}

describe("application route guards", () => {
  beforeEach(() => {
    clearSession("expired");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("redirects an unauthenticated visitor to login with a redirect target", async () => {
    const router = renderRoute("/processing-queue");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/login");
    });
    expect(router.state.location.search).toContain("redirect=%2Fprocessing-queue");
  });

  it("renders the login form for a guest", async () => {
    renderRoute("/login");

    expect(await screen.findByRole("heading", { name: "Sign In" })).toBeInTheDocument();
    expect(screen.getByLabelText("Login Name")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
  });

  it("sends an authenticated visitor away from the login page", async () => {
    await authenticate();
    const router = renderRoute("/login");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/search");
    });
  });

  it("redirects the root path to search", async () => {
    await authenticate();
    const router = renderRoute("/");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/search");
    });
  });

  it("keeps the legacy sources entry path pointing at the browser", async () => {
    await authenticate();
    const router = renderRoute("/sources");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/groups");
    });
  });

  it("redirects the bare settings path to appearance", async () => {
    await authenticate();
    const router = renderRoute("/settings");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/settings/appearance");
    });
  });

  it("redirects a non-admin away from an admin settings section", async () => {
    await authenticate({ is_admin: false });
    const router = renderRoute("/settings/admin-users");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/settings/appearance");
    });
  });

  it("allows an admin into an admin settings section", async () => {
    await authenticate({ is_admin: true });
    const router = renderRoute("/settings/translation");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/settings/translation");
    });
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
  });

  it("keeps the group workspace paths addressable", async () => {
    await authenticate();
    const root = renderRoute("/groups/stock");
    await waitFor(() => {
      expect(root.state.location.pathname).toBe("/groups/stock");
    });
    expect(await screen.findByRole("heading", { name: "Browser" })).toBeInTheDocument();

    const members = renderRoute("/groups/stock/members");
    await waitFor(() => {
      expect(members.state.location.pathname).toBe("/groups/stock/members");
    });

    const settings = renderRoute("/groups/stock/settings");
    await waitFor(() => {
      expect(settings.state.location.pathname).toBe("/groups/stock/settings");
    });
  });

  it("redirects the legacy group overview URL to the group root", async () => {
    await authenticate();
    const router = renderRoute("/groups/stock/overview");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/groups/stock");
    });
  });

  it("preserves an encoded nested group path", async () => {
    await authenticate();
    const router = renderRoute("/groups/stock%2Falpha");

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/groups/stock%2Falpha");
    });
    expect(await screen.findByRole("heading", { name: "Browser" })).toBeInTheDocument();
  });

  it("renders a not-found page for an unknown path", async () => {
    renderRoute("/missing/page");

    expect(await screen.findByRole("heading", { name: "Page not found" })).toBeInTheDocument();
  });
});
