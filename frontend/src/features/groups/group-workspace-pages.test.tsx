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

async function openMembers(role: "owner" | "viewer" | "maintainer") {
  const backend = installGroupBackend({ group: { ...GROUP, current_role: role } });
  await renderAt("/groups/stock%2Falpha/members");
  await screen.findByTestId("group-shell");
  return backend;
}

describe("group members", () => {
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

  it("hides member administration from a viewer by default deny", async () => {
    await openMembers("viewer");

    expect(await screen.findByTestId("members-table")).toBeInTheDocument();
    expect(screen.queryByTestId("members-add")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Delete" })).not.toBeInTheDocument();
  });

  it("adds a member selected from the user directory with its role", async () => {
    const backend = await openMembers("owner");
    const user = userEvent.setup();

    await user.click(await screen.findByTestId("members-add"));
    const candidate = await screen.findByTestId("member-candidate");
    expect(candidate).toHaveTextContent("viewer");
    await user.click(candidate);
    await user.click(screen.getByRole("combobox", { name: "Role" }));
    await user.click(await screen.findByRole("option", { name: "Maintainer" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/groups/by-path/{group_path}/members");
      expect(calls).toHaveLength(1);
      expect(calls[0].params?.path).toEqual({ group_path: "stock/alpha" });
      expect(calls[0].body).toEqual({ login_name: "viewer", role: "maintainer" });
    });
  });

  it("requires confirmation before removing a member", async () => {
    const backend = await openMembers("owner");
    const user = userEvent.setup();

    const row = (await screen.findAllByTestId("members-row"))[1];
    await user.click(within(row).getByRole("button", { name: "Delete" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("Remove member viewer?");

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsFor("DELETE", "/v1/groups/by-path/{group_path}/members/{login_name}")).toHaveLength(0);

    await user.click(within(row).getByRole("button", { name: "Delete" }));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));

    await waitFor(() => {
      const calls = backend.callsFor("DELETE", "/v1/groups/by-path/{group_path}/members/{login_name}");
      expect(calls).toHaveLength(1);
      expect(calls[0].params?.path).toEqual({ group_path: "stock/alpha", login_name: "viewer" });
    });
  });

  it("shows human role labels in the member table and role picker while sending the code", async () => {
    const backend = await openMembers("owner");
    const user = userEvent.setup();

    const rows = await screen.findAllByTestId("members-row");
    expect(rows[0]).toHaveTextContent("Owner");
    expect(rows[1]).toHaveTextContent("Viewer");
    expect(rows[0]).not.toHaveTextContent(/^owner$/);

    await user.click(screen.getByTestId("members-add"));
    await user.click(await screen.findByTestId("member-candidate"));
    await user.click(screen.getByRole("combobox", { name: "Role" }));
    expect(await screen.findByRole("option", { name: "Owner" })).toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "Viewer" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/groups/by-path/{group_path}/members");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({ login_name: "viewer", role: "viewer" });
    });
  });

  it("localizes the group header role and visibility badges", async () => {
    await openMembers("owner");

    const shell = await screen.findByTestId("group-shell");
    const header = shell.querySelector("header") as HTMLElement;
    expect(header).toHaveTextContent("Owner");
    expect(header).toHaveTextContent("Private");
    expect(header).not.toHaveTextContent("owner");
    expect(header).not.toHaveTextContent("private");

    setLocale("zh-CN");
    await waitFor(() => {
      const chineseHeader = screen.getByTestId("group-shell").querySelector("header") as HTMLElement;
      expect(chineseHeader).toHaveTextContent("所有者");
      expect(chineseHeader).toHaveTextContent("私有");
      expect(chineseHeader).not.toHaveTextContent("owner");
    });
    const chineseRows = await screen.findAllByTestId("members-row");
    expect(chineseRows[0]).toHaveTextContent("所有者");
  });

  it("pages and sorts members through the request query", async () => {
    const backend = await openMembers("owner");
    const user = userEvent.setup();

    await screen.findByTestId("members-table");
    await user.click(screen.getByRole("button", { name: /Display Name/ }));

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/members");
      expect(calls.at(-1)?.params?.query).toMatchObject({ sort_by: "display_name", sort_direction: "asc" });
    });
  });
});

describe("group settings", () => {
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

  async function openSettings(role: "owner" | "viewer") {
    const backend = installGroupBackend({
      group: { ...GROUP, current_role: role },
      groups: [GROUP, { ...GROUP, group_id: 3, group_key: "beta", group_path: "beta", name: "Beta" }],
    });
    await renderAt("/groups/stock%2Falpha/settings");
    await screen.findByTestId("group-shell");
    return backend;
  }

  it("saves the group name and visibility with a PATCH body", async () => {
    const backend = await openSettings("owner");
    const user = userEvent.setup();

    const name = await screen.findByTestId("group-settings-name");
    await user.clear(name);
    await user.type(name, "Alpha Renamed");
    await user.click(screen.getAllByRole("button", { name: "Save" })[0]);

    await waitFor(() => {
      const calls = backend.callsFor("PATCH", "/v1/groups/by-path/{group_path}");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({ name: "Alpha Renamed", visibility: "private" });
    });
  });

  it("keeps settings read-only for a viewer", async () => {
    await openSettings("viewer");

    expect(await screen.findByTestId("group-settings")).toBeInTheDocument();
    expect(screen.queryByTestId("group-settings-name")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Move" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Delete" })).not.toBeInTheDocument();
  });

  it("creates a metadata index for the selected source key", async () => {
    const backend = await openSettings("owner");
    const user = userEvent.setup();

    await user.type(await screen.findByTestId("metadata-source-key"), "gov_documents{Enter}");
    await user.click(screen.getByTestId("metadata-add"));
    await user.type(await screen.findByTestId("metadata-path"), "$.region");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/groups/by-path/{group_path}/metadata-indexes");
      expect(calls).toHaveLength(1);
      expect(calls[0].params?.query).toMatchObject({ source_key: "gov_documents" });
      expect(calls[0].body).toEqual({
        path: "$.region",
        data_type: "keyword",
        value_kind: "scalar",
        sortable: false,
      });
    });
  });

  it("moves the group to the selected parent", async () => {
    const backend = await openSettings("owner");
    const user = userEvent.setup();

    await screen.findByTestId("group-settings");
    await user.click(screen.getByRole("button", { name: "Move" }));
    const candidate = (await screen.findAllByTestId("move-group-candidate"))[0];
    expect(candidate).toHaveTextContent("Beta");
    await user.click(candidate);
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Move" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/groups/by-path/{group_path}/move");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({ target_parent_group_path: "beta" });
    });
  });

  it("saves translation defaults including the parsed glossary", async () => {
    const backend = await openSettings("owner");
    const user = userEvent.setup();

    const glossary = await screen.findByLabelText("Glossary (source = target, one per line)");
    await user.type(glossary, "stock = 股票");
    const saveButtons = screen.getAllByRole("button", { name: "Save" });
    await user.click(saveButtons.at(-1) as HTMLElement);

    await waitFor(() => {
      const calls = backend.callsFor("PUT", "/v1/groups/by-path/{group_path}/translation-settings");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toMatchObject({
        enabled: true,
        default_target_locales: ["zh-CN"],
        source_locale: "en-US",
        glossary: [{ source: "stock", target: "股票" }],
      });
    });
  });
});
