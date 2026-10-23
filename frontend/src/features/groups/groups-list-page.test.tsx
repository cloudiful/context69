import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { appRoutes } from "@/app/router";
import { clearSession } from "@/lib/auth-store";
import { setLocale } from "@/lib/preferences-store";

import { installDomPolyfills } from "@/features/search/test-support";

import { authenticate, GROUP, installGroupBackend, TEST_USER } from "./test-support";

async function renderAt(path: string) {
  const router = createMemoryRouter(appRoutes, { initialEntries: [path] });
  render(
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>,
  );
  return router;
}

describe("groups list page", () => {
  beforeAll(() => installDomPolyfills());

  beforeEach(async () => {
    clearSession(null);
    setLocale("en");
    vi.stubGlobal("fetch", vi.fn(async () => new Response(
      JSON.stringify({ user: TEST_USER }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    )));
    await authenticate();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("lists visible groups and navigates into a nested group path", async () => {
    installGroupBackend();
    const router = await renderAt("/groups");

    expect(await screen.findByTestId("groups-row")).toHaveTextContent("Alpha");
    expect(screen.getAllByRole("heading", { name: "Browser" }).length).toBeGreaterThan(0);

    await userEvent.setup().click(screen.getByRole("button", { name: "Open" }));

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/groups/stock%2Falpha");
    });
  });

  it("sends the debounced search text, filters and paging to the list endpoint", async () => {
    const backend = installGroupBackend();
    const user = userEvent.setup();
    await renderAt("/groups");

    await screen.findByTestId("groups-row");
    await user.type(screen.getByTestId("groups-search"), "alpha");

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups");
      expect(calls.at(-1)?.params?.query).toMatchObject({ query: "alpha", page: 1 });
    });

    await user.click(screen.getByRole("combobox", { name: "Visibility" }));
    await user.click(await screen.findByRole("option", { name: "Public" }));

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups");
      expect(calls.at(-1)?.params?.query).toMatchObject({ visibility: "public", page: 1 });
    });
  });

  it("creates a shared group with the typed key, name and visibility", async () => {
    const backend = installGroupBackend();
    const user = userEvent.setup();
    await renderAt("/groups");

    await screen.findByTestId("groups-row");
    await user.click(screen.getByTestId("groups-create"));
    await user.type(await screen.findByTestId("group-entity-key"), "beta");
    await user.type(screen.getByTestId("group-entity-name"), "Beta Team");

    await user.click(screen.getByRole("combobox", { name: "Visibility" }));
    await user.click(await screen.findByRole("option", { name: "Public" }));
    await user.click(screen.getByRole("button", { name: "Create Group" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/groups");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({
        group_key: "beta",
        name: "Beta Team",
        visibility: "public",
        kind: "shared",
      });
    });
  });

  it("renders localized visibility, kind and role labels instead of raw enum codes", async () => {
    installGroupBackend({ groups: [GROUP] });
    await renderAt("/groups");

    const row = await screen.findByTestId("groups-row");
    const cells = within(row).getAllByRole("cell");
    const labels = cells.map((cell) => cell.textContent);
    expect(labels).toEqual(expect.arrayContaining(["Private", "Shared", "Owner"]));
    expect(labels).not.toContain("private");
    expect(labels).not.toContain("shared");
    expect(labels).not.toContain("owner");

    setLocale("zh-CN");
    await waitFor(() => {
      const chinese = within(screen.getByTestId("groups-row")).getAllByRole("cell").map((cell) => cell.textContent);
      expect(chinese).toEqual(expect.arrayContaining(["私有", "共享", "所有者"]));
      expect(chinese).not.toContain("private");
      expect(chinese).not.toContain("owner");
    });
  });

  it("keeps an unknown future enum value visible instead of crashing", async () => {
    installGroupBackend({
      group: {
        ...GROUP,
        visibility: "future_visibility" as never,
        kind: "future_kind" as never,
        current_role: "future_role" as never,
      },
      groups: [{
        ...GROUP,
        visibility: "future_visibility" as never,
        kind: "future_kind" as never,
        current_role: "future_role" as never,
      }],
    });
    await renderAt("/groups");

    const row = await screen.findByTestId("groups-row");
    const text = row.textContent ?? "";
    expect(text).toContain("future_visibility");
    expect(text).toContain("future_kind");
    expect(text).toContain("future_role");
  });

  it("keeps the dialog description free of duplicated field guidance", async () => {
    installGroupBackend();
    const user = userEvent.setup();
    await renderAt("/groups");

    await screen.findByTestId("groups-row");
    await user.click(screen.getByTestId("groups-create"));
    const dialog = await screen.findByRole("dialog");

    expect(within(dialog).getByRole("heading", { name: "Create Group" })).toBeInTheDocument();
    expect(dialog).not.toHaveTextContent("Group name and visibility.");
  });

  it("reports a failed group load and retries it", async () => {
    const backend = installGroupBackend({ groups: [GROUP] });
    backend.spies.GET.mockImplementationOnce((() => Promise.resolve({
      error: { message: "group list unavailable" },
      response: { ok: false, status: 500 },
    })) as never);
    const user = userEvent.setup();
    await renderAt("/groups");

    expect(await screen.findByRole("alert")).toHaveTextContent("group list unavailable");
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByTestId("groups-row")).toBeInTheDocument();
  });
});
