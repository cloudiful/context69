import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { AppShell } from "./app-shell";
import { authSessionStore, clearSession, restoreSession } from "@/lib/auth-store";
import { setLocale } from "@/lib/preferences-store";
import { installMockStorage } from "@/test-utils/storage";

const user = {
  user_id: 3,
  login_name: "planner",
  display_name: "Planner",
  is_admin: false,
  personal_group_path: "personal-planner",
};

async function authenticate() {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ user }), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  })));
  await restoreSession();
}

function renderShell(path = "/search") {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: <AppShell />,
        children: [
          { path: "search", element: <h1>Search area</h1> },
          { path: "processing-queue", element: <h1>Queue area</h1> },
          { path: "groups", element: <h1>Groups area</h1> },
          { path: "settings", element: <h1>Settings area</h1> },
        ],
      },
      { path: "/login", element: <h1>Sign In</h1> },
    ],
    { initialEntries: [path] },
  );

  render(
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>,
  );

  return router;
}

describe("authenticated shell", () => {
  beforeEach(() => {
    installMockStorage();
    setLocale("en");
    clearSession(null);
    window.innerWidth = 1024;
  });

  afterEach(() => {
    window.innerWidth = 1024;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("shows every product area with the current one marked", async () => {
    await authenticate();
    renderShell("/search");

    const nav = await screen.findByRole("navigation", { name: "Content Search" });
    const searchLink = within(nav).getByRole("link", { name: "Search" });
    expect(searchLink).toHaveAttribute("aria-current", "page");
    expect(within(nav).getByRole("link", { name: "Processing Queue" })).toBeInTheDocument();
    expect(within(nav).getByRole("link", { name: "Browser" })).toBeInTheDocument();
    expect(within(nav).getByRole("link", { name: "Settings" })).toBeInTheDocument();
  });

  it("renders the area screen inside the shell", async () => {
    await authenticate();
    renderShell("/processing-queue");

    expect(await screen.findByRole("heading", { name: "Queue area" })).toBeInTheDocument();
  });

  it("scopes a compact desktop sidebar width to the app shell", async () => {
    await authenticate();
    renderShell("/search");

    const nav = await screen.findByRole("navigation", { name: "Content Search" });
    const wrapper = nav.closest<HTMLElement>('[data-slot="sidebar-wrapper"]');
    expect(wrapper).not.toBeNull();
    expect(wrapper?.style.getPropertyValue("--sidebar-width")).toBe("13rem");
    expect(within(nav).getAllByRole("link")).toHaveLength(4);
  });

  it("switches the navigation labels when the locale changes", async () => {
    await authenticate();
    renderShell("/search");

    expect(await screen.findByRole("link", { name: "Browser" })).toBeInTheDocument();

    setLocale("zh-CN");

    expect(await screen.findByRole("link", { name: "浏览器" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "处理队列" })).toBeInTheDocument();
  });

  it("opens the mobile navigation from the keyboard and closes on selection", async () => {
    window.innerWidth = 390;
    await authenticate();
    renderShell("/search");

    const trigger = await screen.findByRole("button", { name: "Show" });
    trigger.focus();
    await userEvent.keyboard("{Enter}");

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("link", { name: "Settings" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(await screen.findByRole("heading", { name: "Settings area" })).toBeInTheDocument();
  });

  it("collapses the sidebar from the trigger on a wide viewport", async () => {
    await authenticate();
    renderShell("/search");

    const trigger = await screen.findByRole("button", { name: "Hide" });
    await userEvent.click(trigger);

    expect(await screen.findByRole("button", { name: "Show" })).toBeInTheDocument();
  });

  it("logs out from the user menu and returns to the login page", async () => {
    await authenticate();
    renderShell("/search");

    await userEvent.click(await screen.findByRole("button", { name: "Planner" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Log Out" }));

    expect(await screen.findByRole("heading", { name: "Sign In" })).toBeInTheDocument();
    expect(authSessionStore.getSnapshot().user).toBeNull();
  });
});
