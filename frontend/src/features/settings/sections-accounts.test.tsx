import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "@/services/api";
import { setLocale } from "@/lib/preferences-store";
import { installMockStorage } from "@/test-utils/storage";

import {
  ADMIN_USER_ROW,
  installDomPolyfills,
  installFakeBackend,
  openSettings,
  resetAuth,
  resetPreferences,
  resetToasts,
} from "./test-support";

describe("access tokens settings", () => {
  beforeAll(() => installDomPolyfills());
  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    resetAuth();
  });
  afterEach(() => {
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("requires a name before a token can be created", async () => {
    installFakeBackend();
    await openSettings("/settings/access-tokens");

    expect(await screen.findByText("cli")).toBeInTheDocument();
    expect(screen.getByTestId("personal-access-token-create")).toBeDisabled();

    await userEvent.type(screen.getByTestId("personal-access-token-name"), "deploy");
    expect(screen.getByTestId("personal-access-token-create")).toBeEnabled();

    await userEvent.click(screen.getByTestId("pat-scope-search"));
    expect(screen.getByTestId("personal-access-token-create")).toBeDisabled();
  });

  it("creates a token, reveals it once and copies it", async () => {
    const backend = installFakeBackend();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    await openSettings("/settings/access-tokens");

    await userEvent.type(await screen.findByTestId("personal-access-token-name"), "deploy");
    await userEvent.click(screen.getByTestId("personal-access-token-create"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/auth/personal-access-tokens")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/auth/personal-access-tokens")[0].body).toEqual({
      name: "deploy",
      scopes: ["search"],
      expires_in_days: 30,
    });

    const secret = await screen.findByTestId("personal-access-token-secret");
    expect(secret).toHaveValue("c69_revealed_once");

    await userEvent.click(screen.getByRole("button", { name: "Copy Token" }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith("c69_revealed_once");
    });

    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByTestId("personal-access-token-secret")).not.toBeInTheDocument();
    expect(JSON.stringify({ ...window.localStorage })).not.toContain("c69_revealed_once");
  });

  it("shows a localized lifetime for every expiry option in both locales", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/access-tokens");

    const trigger = await screen.findByTestId("personal-access-token-expiry");
    expect(trigger).toHaveTextContent("30 days");
    await userEvent.click(trigger);
    const options = await screen.findAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual([
      "7 days",
      "30 days",
      "90 days",
      "365 days",
    ]);
    expect(options.every((option) => !/expiry\d+Days/.test(option.textContent ?? ""))).toBe(true);

    // Pick the 90-day lifetime, then confirm the same choice in Chinese.
    await userEvent.click(screen.getByRole("option", { name: "90 days" }));
    expect(trigger).toHaveTextContent("90 days");

    setLocale("zh-CN");
    await waitFor(() => expect(screen.getByTestId("personal-access-token-expiry")).toHaveTextContent("90天"));
    const chineseTrigger = screen.getByTestId("personal-access-token-expiry");
    await userEvent.click(chineseTrigger);
    const chineseOptions = await screen.findAllByRole("option");
    expect(chineseOptions.map((option) => option.textContent)).toEqual([
      "7天",
      "30天",
      "90天",
      "365天",
    ]);
    await userEvent.keyboard("{Escape}");
    setLocale("en");
    await waitFor(() => expect(screen.getByTestId("personal-access-token-expiry")).toHaveTextContent("90 days"));

    // The label is translated; the request keeps the numeric lifetime.
    await userEvent.type(screen.getByTestId("personal-access-token-name"), "quarter");
    await userEvent.click(screen.getByTestId("personal-access-token-create"));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/auth/personal-access-tokens")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/auth/personal-access-tokens")[0].body).toEqual({
      name: "quarter",
      scopes: ["search"],
      expires_in_days: 90,
    });
  });

  it("revokes a token only after the confirmation is accepted", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/access-tokens");

    await userEvent.click(await screen.findByRole("button", { name: "Revoke" }));
    let dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsFor("DELETE", "/v1/auth/personal-access-tokens/{token_id}")).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "Revoke" }));
    dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Revoke" }));

    await waitFor(() => {
      expect(backend.callsFor("DELETE", "/v1/auth/personal-access-tokens/{token_id}")).toHaveLength(1);
    });
    expect(await screen.findByText("Revoked")).toBeInTheDocument();
  });
});

describe("translation settings", () => {
  beforeAll(() => installDomPolyfills());
  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    resetAuth();
  });
  afterEach(() => {
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("saves provider enablement and priority through the global Save", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/translation");

    expect(await screen.findByText("DeepL")).toBeInTheDocument();
    expect(screen.getByText("LibreTranslate")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("switch", { name: "DeepL" }));
    await userEvent.click(screen.getAllByRole("button", { name: "Move" })[2]);
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/translation")).toHaveLength(1);
    });
    const providers = (backend.callsFor("PUT", "/v1/settings/translation")[0].body as {
      providers: { provider: string; enabled: boolean; priority: number; api_key?: string }[];
    }).providers;
    expect(providers.find((provider) => provider.provider === "deepl")?.enabled).toBe(true);
    expect(providers.map((provider) => provider.provider)).toEqual(["llm", "deepl", "libretranslate"]);
    expect(providers.map((provider) => provider.priority)).toEqual([0, 1, 2]);
    expect(providers.every((provider) => provider.api_key === undefined)).toBe(true);
  });

  it("edits a provider through the dialog and probes the stored key", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/translation");

    const editButtons = await screen.findAllByRole("button", { name: "Edit" });
    await userEvent.click(editButtons[1]);

    const dialog = await screen.findByRole("dialog");
    const endpoint = within(dialog).getByLabelText("Endpoint");
    await userEvent.clear(endpoint);
    await userEvent.type(endpoint, "https://openrouter.ai/api/v2");

    await userEvent.click(within(dialog).getByRole("button", { name: "Test model call" }));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/translation/providers/llm/test")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/settings/translation/providers/llm/test")[0].body).toEqual({
      endpoint: "https://openrouter.ai/api/v2",
      model: "openai/gpt-4.1-mini",
      llm_api_kind: "openai_chat_completions",
      api_key: undefined,
    });
    expect(await within(dialog).findByText("Model call succeeded")).toBeInTheDocument();
    expect(within(dialog).getByText(/Blank key uses the stored key/)).toBeInTheDocument();

    await userEvent.click(within(dialog).getByRole("button", { name: "Confirm" }));
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/translation")).toHaveLength(1);
    });
    const providers = (backend.callsFor("PUT", "/v1/settings/translation")[0].body as {
      providers: { provider: string; endpoint: string | null }[];
    }).providers;
    expect(providers.find((provider) => provider.provider === "llm")?.endpoint).toBe("https://openrouter.ai/api/v2");
  });
});

describe("admin users settings", () => {
  it("keeps a persistent read failure, retries, distinguishes no matches and reloads on clear", async () => {
    const backend = installFakeBackend({ adminUsersFailReads: 1 });
    await openSettings("/settings/admin-users");
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("users unavailable");
    expect(screen.queryByText("No users yet.")).not.toBeInTheDocument();
    toast.dismiss();
    expect(alert).toBeInTheDocument();
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("planner")).toBeInTheDocument();
    const search = screen.getByRole("textbox", { name: "Login Name" });
    await userEvent.type(search, "absent");
    expect(await screen.findByText("No matching users.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await userEvent.clear(search);
    expect(await screen.findByText("planner")).toBeInTheDocument();
    expect(backend.callsFor("GET", "/v1/admin/users").at(-1)?.params).toMatchObject({ query: { page: 1 } });
  });

  it("preserves stale rows on refetch failure and retries the selected sort", async () => {
    const options = { adminUsersFailReads: 0 };
    const backend = installFakeBackend(options);
    await openSettings("/settings/admin-users");
    await screen.findByText("planner");
    options.adminUsersFailReads = 1;
    await userEvent.click(screen.getByRole("button", { name: "Display Name" }));
    const alert = await screen.findByRole("alert");
    expect(screen.getByText("planner")).toBeInTheDocument();
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(backend.callsFor("GET", "/v1/admin/users").at(-1)?.params).toMatchObject({ query: { sort_by: "display_name", sort_direction: "asc" } });
  });

  it.each(["failure", "success"])("ignores an outdated user load %s after the latest search succeeds", async (outcome) => {
    installFakeBackend();
    await openSettings("/settings/admin-users");
    await screen.findByText("planner");
    let rejectOld: (error: Error) => void = () => {};
    let resolveOld: (value: Awaited<ReturnType<typeof apiClient.listAdminUsers>>) => void = () => {};
    const spy = vi.spyOn(apiClient, "listAdminUsers").mockImplementationOnce(() => new Promise((resolve, reject) => { rejectOld = reject; resolveOld = resolve; }));
    const search = screen.getByRole("textbox", { name: "Login Name" });
    await userEvent.type(search, "old");
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(1));
    await userEvent.clear(search);
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(2));
    await act(async () => {
      if (outcome === "failure") rejectOld(new Error("old failure"));
      else resolveOld({ items: [], pagination: { page: 9, page_size: 10, total: 0, total_pages: 0 } });
    });
    expect(screen.getByText("planner")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows successful empty users without a load failure", async () => {
    installFakeBackend({ users: [] });
    await openSettings("/settings/admin-users");
    expect(await screen.findByText("No users yet.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  beforeAll(() => installDomPolyfills());
  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    resetAuth();
  });
  afterEach(() => {
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("creates, edits, resets and disables an admin account", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/admin-users");

    expect(await screen.findByText("planner")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Create User" }));
    let dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Login Name"), "auditor");
    await userEvent.type(within(dialog).getByLabelText("Display Name"), "Auditor");
    await userEvent.type(within(dialog).getByLabelText("Password"), "secret-pass");
    await userEvent.click(within(dialog).getByRole("switch", { name: "Administrator" }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Create User" }));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/admin/users")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/admin/users")[0].body).toEqual({
      login_name: "auditor",
      display_name: "Auditor",
      password: "secret-pass",
      is_admin: true,
    });
    expect(await screen.findByText("auditor")).toBeInTheDocument();

    await userEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    dialog = await screen.findByRole("dialog");
    await userEvent.clear(within(dialog).getByLabelText("Display Name"));
    await userEvent.type(within(dialog).getByLabelText("Display Name"), "Planner Renamed");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(backend.callsFor("PATCH", "/v1/admin/users/{login_name}")).toHaveLength(1);
    });
    expect(backend.callsFor("PATCH", "/v1/admin/users/{login_name}")[0].body).toMatchObject({
      display_name: "Planner Renamed",
    });

    await userEvent.click(screen.getAllByRole("button", { name: "Reset Password" })[0]);
    dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("New Password"), "another-pass");
    await userEvent.click(within(dialog).getByRole("button", { name: "Reset Password" }));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/admin/users/{login_name}/reset-password")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/admin/users/{login_name}/reset-password")[0].body).toEqual({
      password: "another-pass",
    });

    await userEvent.click(screen.getAllByRole("button", { name: "Disable User" })[0]);
    const confirm = await screen.findByRole("alertdialog");
    expect(within(confirm).getByText("Disable user planner?")).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: "Disable User" }));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/admin/users/{login_name}/disable")).toHaveLength(1);
    });
    expect(await screen.findByRole("button", { name: "Enable User" })).toBeInTheDocument();
  });

  it("searches accounts through the debounced query", async () => {
    const backend = installFakeBackend({ users: [ADMIN_USER_ROW] });
    await openSettings("/settings/admin-users");

    await userEvent.type(await screen.findByRole("textbox", { name: "Login Name" }), "plan");

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/admin/users");
      expect(calls.some((call) => (call.params as { query: { query?: string } }).query.query === "plan")).toBe(true);
    });
  });
});
