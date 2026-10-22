import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { AppToaster } from "@/app/toaster";
import { LoginPage } from "./login-page";
import { authSessionStore, clearSession } from "@/lib/auth-store";

const user = {
  user_id: 1,
  login_name: "admin",
  display_name: "Administrator",
  is_admin: true,
  personal_group_path: "personal-admin",
};

function renderAt(initialEntry: string) {
  const router = createMemoryRouter(
    [
      { path: "/login", element: <LoginPage /> },
      { path: "/search", element: <h1>Search area</h1> },
    ],
    { initialEntries: [initialEntry] },
  );

  renderAtRouter(router);
  return router;
}

function renderAtRouter(router: ReturnType<typeof createMemoryRouter>) {
  render(
    <AppProviders>
      <RouterProvider router={router} />
      <AppToaster />
    </AppProviders>,
  );
}

describe("login page", () => {
  beforeEach(() => {
    clearSession(null);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("requires both credential fields before calling the API", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    renderAt("/login");

    await userEvent.click(screen.getByRole("button", { name: "Sign In" }));

    expect(await screen.findByText("Login Name is required.")).toBeInTheDocument();
    expect(screen.getByText("Password is required.")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Login Name")).toHaveAttribute("aria-invalid", "true");
  });

  it("reports invalid credentials without leaving the page", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ message: "Denied" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    })));
    const router = renderAt("/login");

    await userEvent.type(screen.getByLabelText("Login Name"), "admin");
    await userEvent.type(screen.getByLabelText("Password"), "nope");
    await userEvent.click(screen.getByRole("button", { name: "Sign In" }));

    expect(await screen.findByText("Invalid login name or password.")).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/login");
    expect(authSessionStore.getSnapshot().user).toBeNull();
  });

  it("signs in with the cookie session and follows the redirect target", async () => {
    vi.stubGlobal("fetch", vi.fn()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ user }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })));
    const router = renderAt("/login?redirect=%2Fsearch");

    await userEvent.type(screen.getByLabelText("Login Name"), "  admin  ");
    await userEvent.type(screen.getByLabelText("Password"), "secret");
    await userEvent.click(screen.getByRole("button", { name: "Sign In" }));

    expect(await screen.findByRole("heading", { name: "Search area" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/search");
    expect(authSessionStore.getSnapshot().user).toEqual(user);
  });

  it("ignores an off-site redirect target", async () => {
    vi.stubGlobal("fetch", vi.fn()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ user }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })));
    const router = renderAt("/login?redirect=%2F%2Fevil.example");

    await userEvent.type(screen.getByLabelText("Login Name"), "admin");
    await userEvent.type(screen.getByLabelText("Password"), "secret");
    await userEvent.click(screen.getByRole("button", { name: "Sign In" }));

    expect(router.state.location.pathname).toBe("/search");
  });

  it("shows the session-unavailable notice for an expired session", async () => {
    renderAt("/login?reason=expired");

    expect(await screen.findByText("Authentication unavailable")).toBeInTheDocument();
  });

  it("locks the sign-in action while the request is in flight", async () => {
    let resolveFetch: (value: Response) => void = () => undefined;
    vi.stubGlobal("fetch", vi.fn(() => new Promise<Response>((resolve) => {
      resolveFetch = resolve;
    })));
    renderAt("/login");

    await userEvent.type(screen.getByLabelText("Login Name"), "admin");
    await userEvent.type(screen.getByLabelText("Password"), "secret");
    await userEvent.click(screen.getByRole("button", { name: "Sign In" }));

    const busyButton = await screen.findByRole("button", { name: "Signing In..." });
    expect(busyButton).toBeDisabled();
    expect(busyButton).toHaveAttribute("aria-busy", "true");

    resolveFetch(new Response(null, { status: 401 }));
  });
});
