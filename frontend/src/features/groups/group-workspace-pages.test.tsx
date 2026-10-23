import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { appRoutes } from "@/app/router";
import { clearSession } from "@/lib/auth-store";
import { setLocale } from "@/lib/preferences-store";
import { installDomPolyfills } from "@/features/search/test-support";
import type { GroupResponse, LibraryTreeResponse } from "@/services/api";

import { authenticate, GROUP, installGroupBackend, TEST_USER, type GroupBackendOptions } from "./test-support";

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

const CHILD_ALPHA: GroupResponse = {
  group_id: 11,
  group_key: "alpha-child",
  group_path: "stock/alpha/alpha-child",
  name: "Alpha Child",
  visibility: "private",
  kind: "shared",
  current_role: "owner",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};

const CHILD_BETA: GroupResponse = {
  ...CHILD_ALPHA,
  group_id: 12,
  group_key: "beta-child",
  group_path: "stock/beta/beta-child",
  name: "Beta Child",
};

const EMPTY_TREE: LibraryTreeResponse = {
  root: {
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    folder_id: null,
    parent_folder_id: null,
    name: "Alpha",
    path: "Alpha",
    processing_count: 0,
    children: [],
    files: [],
  },
};

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

  it("does not call a failed member read empty and retries the current search", async () => {
    const options: GroupBackendOptions = { membersFailReads: 1 };
    const backend = installGroupBackend(options);
    await renderAt("/groups/stock%2Falpha/members");
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Failed to load members.");
    expect(screen.queryByText("No members yet.")).not.toBeInTheDocument();
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("viewer")).toBeInTheDocument();
    options.membersFailReads = 1;
    await userEvent.type(screen.getByTestId("members-search"), "z");
    const failedSearch = await screen.findByRole("alert");
    expect(screen.queryByText("No matching members.")).not.toBeInTheDocument();
    await userEvent.click(within(failedSearch).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("No matching members.")).toBeInTheDocument();
    expect(backend.callsFor("GET", "/v1/groups/by-path/{group_path}/members").at(-1)?.params?.query).toMatchObject({ query: "z", page: 1, page_size: 50 });
  });

  it("shows a retryable child failure without a false empty folder", async () => {
    const backend = installGroupBackend({ childrenFailReads: 1, children: [CHILD_ALPHA], tree: EMPTY_TREE });
    await renderAt("/groups/stock%2Falpha");
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Failed to load child groups.");
    expect(screen.queryByTestId("library-empty")).not.toBeInTheDocument();
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Alpha Child")).toBeInTheDocument();
    expect(backend.callsFor("GET", "/v1/groups/by-path/{group_path}/children")).toHaveLength(2);
  });

  it("keeps child rows and a local error when their same-query refresh fails", async () => {
    const options: GroupBackendOptions = { children: [CHILD_ALPHA], tree: EMPTY_TREE };
    const backend = installGroupBackend(options);
    await renderAt("/groups/stock%2Falpha");
    const row = await screen.findByTestId("library-group-row");
    await userEvent.click(within(row).getByRole("button", { name: "Actions" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Edit" }));
    options.childrenFailReads = 1;
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(screen.getByTestId("library-group-row")).toHaveTextContent("Alpha Child");
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(backend.callsFor("GET", "/v1/groups/by-path/{group_path}/children").at(-1)?.params?.query).toMatchObject({ page: 1, page_size: 50 });
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

  it("keeps cached members marked failed when the post-command refresh fails", async () => {
    const options: GroupBackendOptions = {};
    installGroupBackend(options);
    await renderAt("/groups/stock%2Falpha/members");
    const row = (await screen.findAllByTestId("members-row"))[1];
    options.membersFailReads = 1;
    await userEvent.click(within(row).getByRole("button", { name: "Delete" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    const alert = await screen.findByRole("alert");
    expect(screen.getByText("viewer")).toBeInTheDocument();
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.queryByText("viewer")).not.toBeInTheDocument());
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
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
    await waitFor(() => {
      expect(glossary).toBeEnabled();
    });
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

describe("group child search", () => {
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

  async function openOverview(options: GroupBackendOptions = {}) {
    const backend = installGroupBackend({ tree: EMPTY_TREE, ...options });
    const router = await renderAt("/groups/stock%2Falpha");
    await screen.findByTestId("group-shell");
    return { backend, router };
  }

  it("keeps the search control focused while a search has no match", async () => {
    const { backend } = await openOverview({ children: [CHILD_ALPHA] });
    const user = userEvent.setup();

    const search = await screen.findByTestId("child-group-search");
    await user.type(search, "zzz");

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/children");
      expect(calls.at(-1)?.params?.query).toMatchObject({ query: "zzz" });
    });
    await waitFor(() => {
      expect(screen.queryByTestId("library-group-row")).not.toBeInTheDocument();
    });

    const control = screen.getByTestId("child-group-search");
    expect(control).toHaveValue("zzz");
    expect(control).toHaveFocus();
    expect(screen.getByTestId("library-empty")).toHaveTextContent("No matching child groups.");
  });

  it("keeps the control mounted while the next search loads and restores the rows when it is cleared", async () => {
    const { backend } = await openOverview({ children: [CHILD_ALPHA] });
    const user = userEvent.setup();

    await screen.findByTestId("library-group-row");
    const reads = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/children").length;
    backend.blockChildren();
    await user.type(screen.getByTestId("child-group-search"), "beta");
    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/children");
      expect(calls.length).toBeGreaterThan(reads);
      expect(calls.at(-1)?.params?.query).toMatchObject({ query: "beta" });
    });

    expect(screen.getByTestId("library-group-row")).toHaveTextContent("Alpha Child");
    expect(screen.getByTestId("child-group-search")).toHaveValue("beta");

    backend.releaseChildren();
    await waitFor(() => {
      expect(screen.queryByTestId("library-group-row")).not.toBeInTheDocument();
    });
    expect(screen.getByTestId("child-group-search")).toHaveFocus();

    await user.clear(screen.getByTestId("child-group-search"));
    await waitFor(() => {
      expect(screen.getByTestId("library-group-row")).toHaveTextContent("Alpha Child");
    });
  });

  it("never carries the children of another group into a group switch", async () => {
    const { backend, router } = await openOverview({
      childrenByGroupPath: { "stock/alpha": [CHILD_ALPHA], "stock/beta": [CHILD_BETA] },
    });

    await screen.findByTestId("library-group-row");
    backend.blockChildren();
    await router.navigate("/groups/stock%2Fbeta");
    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/children");
      expect(calls.at(-1)?.params?.path).toEqual({ group_path: "stock/beta" });
    });

    expect(screen.queryByText("Alpha Child")).not.toBeInTheDocument();
    expect(screen.queryByText("Beta Child")).not.toBeInTheDocument();

    backend.releaseChildren();
    await waitFor(() => {
      expect(screen.getByTestId("library-group-row")).toHaveTextContent("Beta Child");
    });
    expect(screen.queryByText("Alpha Child")).not.toBeInTheDocument();
  });
});

describe("group translation defaults", () => {
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

  const LOADED = {
    enabled: true,
    default_target_locales: ["zh-CN", "ja-JP"],
    source_locale: "en-US",
    glossary: [{ source: "stock", target: "股票" }],
    queued_count: 0,
    running_count: 0,
    succeeded_count: 3,
    failed_count: 0,
  };

  async function openTranslation(options: GroupBackendOptions = {}, role: "owner" | "viewer" = "owner") {
    const backend = installGroupBackend({ group: { ...GROUP, current_role: role }, ...options });
    const router = await renderAt("/groups/stock%2Falpha/settings");
    await screen.findByTestId("group-translation-settings");
    return { backend, router };
  }

  it("keeps every field and the save disabled when the first load fails", async () => {
    const { backend } = await openTranslation({ translationFailReads: 1 });

    await screen.findByTestId("group-translation-retry");
    expect(screen.getByLabelText("Target locales")).toBeDisabled();
    expect(screen.getByLabelText("Default source locale")).toBeDisabled();
    expect(screen.getByLabelText("Glossary (source = target, one per line)")).toBeDisabled();
    expect(screen.getByRole("switch", { name: "Translate newly ingested documents" })).toBeDisabled();
    expect(screen.getByTestId("group-translation-save")).toBeDisabled();
    expect(backend.callsFor("PUT", "/v1/groups/by-path/{group_path}/translation-settings")).toHaveLength(0);
  });

  it("recovers through the retry and saves the loaded values", async () => {
    const { backend } = await openTranslation({ translationFailReads: 1, translation: LOADED });
    const user = userEvent.setup();

    await user.click(await screen.findByTestId("group-translation-retry"));

    const source = await screen.findByLabelText("Default source locale");
    await waitFor(() => {
      expect(source).toBeEnabled();
    });
    expect(screen.getByLabelText("Target locales")).toHaveValue("zh-CN, ja-JP");
    expect(source).toHaveValue("en-US");
    expect(screen.getByLabelText("Glossary (source = target, one per line)")).toHaveValue("stock = 股票");

    await user.clear(source);
    await user.type(source, "en-GB");
    await user.click(screen.getByTestId("group-translation-save"));

    await waitFor(() => {
      const calls = backend.callsFor("PUT", "/v1/groups/by-path/{group_path}/translation-settings");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({
        enabled: true,
        default_target_locales: ["zh-CN", "ja-JP"],
        source_locale: "en-GB",
        glossary: [{ source: "stock", target: "股票" }],
      });
    });
  });

  it("keeps the edited values when the same group is read again", async () => {
    const { backend } = await openTranslation({
      translationSequence: [LOADED, { ...LOADED, source_locale: "fr-FR" }],
    });
    const user = userEvent.setup();

    const source = await screen.findByLabelText("Default source locale");
    await waitFor(() => {
      expect(source).toBeEnabled();
    });
    await user.clear(source);
    await user.type(source, "en-GB");
    await user.click(screen.getByTestId("group-translation-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/groups/by-path/{group_path}/translation-settings")).toHaveLength(1);
    });
    await waitFor(() => {
      expect(backend.callsFor("GET", "/v1/groups/by-path/{group_path}/translation-settings").length).toBeGreaterThanOrEqual(2);
    });

    expect(source).toHaveValue("en-GB");
  });

  it("keeps the translation form read-only for a viewer", async () => {
    await openTranslation({ translation: LOADED }, "viewer");

    await waitFor(() => {
      expect(screen.getByLabelText("Target locales")).toBeDisabled();
    });
    expect(screen.getByLabelText("Default source locale")).toBeDisabled();
    expect(screen.queryByTestId("group-translation-save")).not.toBeInTheDocument();
  });

  it("loads the values of the group that is opened, never of the former one", async () => {
    const { router } = await openTranslation({
      translationByGroupPath: {
        "stock/alpha": LOADED,
        "stock/beta": { ...LOADED, enabled: false, default_target_locales: ["ja-JP"], source_locale: "de-DE" },
      },
    });

    await waitFor(() => {
      expect(screen.getByLabelText("Default source locale")).toHaveValue("en-US");
    });

    await router.navigate("/groups/stock%2Fbeta/settings");

    await waitFor(() => {
      expect(screen.getByLabelText("Default source locale")).toHaveValue("de-DE");
    });
    expect(screen.getByLabelText("Target locales")).toHaveValue("ja-JP");
    expect(screen.getByRole("switch", { name: "Translate newly ingested documents" })).not.toBeChecked();
  });
});
