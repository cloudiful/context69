import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "@/test-utils/storage";

import {
  installDomPolyfills,
  installFakeBackend,
  openSettings,
  resetAuth,
  resetPreferences,
  resetToasts,
  type FakeBackendOptions,
} from "./test-support";

function settingsNav() {
  return screen.getByRole("navigation", { name: "Settings" });
}

async function goToSection(router: { state: { location: { pathname: string } } }, label: string, path: string) {
  await userEvent.click(within(settingsNav()).getByRole("link", { name: label }));
  const dialog = screen.queryByRole("alertdialog");
  if (dialog) {
    await userEvent.click(within(dialog).getByRole("button", { name: "Leave" }));
  }
  await waitFor(() => {
    expect(router.state.location.pathname).toBe(path);
  });
}

function setup(options: FakeBackendOptions = {}) {
  return installFakeBackend(options);
}

/** Accept the gateway/model compatibility prompt a runtime identity change opens. */
async function acceptCompatibility() {
  const dialog = await screen.findByRole("alertdialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "Keep vectors and save" }));
}

describe("settings save orchestration", () => {
  beforeAll(() => installDomPolyfills());

  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    resetAuth();
  });

  afterEach(() => {
    cleanup();
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("stops at the runtime admission step when the runtime save fails", async () => {
    const backend = installFakeBackend({ fail: { runtimePut: true } });
    const router = await openSettings("/settings/runtime");

    await userEvent.type(screen.getByLabelText("Embedding Model"), "-2");
    await goToSection(router, "Search", "/settings/search");
    await userEvent.clear(screen.getByLabelText("Rerank Candidates"));
    await userEvent.type(screen.getByLabelText("Rerank Candidates"), "55");

    await userEvent.click(screen.getByTestId("settings-save"));
    await acceptCompatibility();

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    expect(backend.callsFor("PUT", "/v1/settings/search")).toHaveLength(0);
    expect(backend.callsFor("PUT", "/v1/settings/docling")).toHaveLength(0);
    expect(backend.callsFor("PUT", "/v1/settings/translation")).toHaveLength(0);
    expect(screen.queryByText("Settings saved")).not.toBeInTheDocument();
  });

  it("captures every group's payload at click time and adopts the saved baselines", async () => {
    const backend = setup();
    const router = await openSettings("/settings/runtime");

    await userEvent.clear(screen.getByLabelText("Embedding Model"));
    await userEvent.type(screen.getByLabelText("Embedding Model"), "embed-2");

    await goToSection(router, "Search", "/settings/search");
    await userEvent.clear(screen.getByLabelText("Rerank Candidates"));
    await userEvent.type(screen.getByLabelText("Rerank Candidates"), "55");

    await goToSection(router, "Translation", "/settings/translation");
    await userEvent.click(await screen.findByRole("switch", { name: "DeepL" }));

    await goToSection(router, "Docling", "/settings/docling");
    await userEvent.clear(screen.getByLabelText("Base URL"));
    await userEvent.type(screen.getByLabelText("Base URL"), "http://docling-2:5001");

    await goToSection(router, "Appearance", "/settings/appearance");
    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Dark" }));

    await userEvent.click(screen.getByTestId("settings-save"));
    await acceptCompatibility();

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });

    const runtimeBody = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      embedding: { model: string };
    };
    const searchBody = backend.callsFor("PUT", "/v1/settings/search")[0].body as {
      candidate_limit: number;
      api_key: unknown;
    };
    const translationBody = backend.callsFor("PUT", "/v1/settings/translation")[0].body as {
      providers: { provider: string; enabled: boolean }[];
    };
    const doclingBody = backend.callsFor("PUT", "/v1/settings/docling")[0].body as {
      connection: { base_url: string };
    };

    expect(runtimeBody.embedding.model).toBe("embed-2");
    expect(searchBody.candidate_limit).toBe(55);
    expect(searchBody.api_key).toEqual({ op: "keep" });
    expect(translationBody.providers.find((provider) => provider.provider === "deepl")?.enabled).toBe(true);
    expect(doclingBody.connection.base_url).toBe("http://docling-2:5001");
    expect(window.localStorage.getItem("context69.theme")).toBe("dark");

    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeDisabled();
    });
  });

  it("adopts successful peers independently and keeps a failed group dirty", async () => {
    const backend = installFakeBackend({ fail: { doclingPut: true } });
    const router = await openSettings("/settings/docling");

    await userEvent.clear(screen.getByLabelText("Base URL"));
    await userEvent.type(screen.getByLabelText("Base URL"), "http://docling-2:5001");

    await goToSection(router, "Search", "/settings/search");
    await userEvent.clear(screen.getByLabelText("Rerank Candidates"));
    await userEvent.type(screen.getByLabelText("Rerank Candidates"), "55");

    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/docling")).toHaveLength(1);
    });
    expect(backend.callsFor("PUT", "/v1/settings/search")).toHaveLength(1);

    await waitFor(() => {
      expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
    });
    expect(screen.getByTestId("settings-save")).toBeEnabled();
    expect(screen.queryByText("Settings saved")).not.toBeInTheDocument();
  });

  it("does not claim success while a newer edit remains after the save", async () => {
    const backend = setup();
    const router = await openSettings("/settings/runtime");
    const putSpy = backend.spies.PUT as unknown as {
      getMockImplementation: () => (path: string, init?: unknown) => unknown;
      mockImplementation: (implementation: (path: string, init?: unknown) => unknown) => void;
    };
    const originalPut = putSpy.getMockImplementation();
    let resolveRuntime: (value: unknown) => void = () => undefined;

    putSpy.mockImplementation((path, init) => {
      if (path === "/v1/settings/runtime") {
        return new Promise((resolve) => {
          resolveRuntime = resolve;
        });
      }
      return originalPut(path, init);
    });

    await userEvent.clear(screen.getByLabelText("Embedding Model"));
    await userEvent.type(screen.getByLabelText("Embedding Model"), "embed-2");
    await userEvent.click(screen.getByTestId("settings-save"));
    await acceptCompatibility();

    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeDisabled();
    });

    await userEvent.clear(screen.getByLabelText("Embedding Model"));
    await userEvent.type(screen.getByLabelText("Embedding Model"), "embed-3");

    resolveRuntime({
      data: {
        ...backend.state.runtime,
        embedding: { ...backend.state.runtime.embedding, model: "embed-2" },
      },
      response: { ok: true, status: 200 },
    });

    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeEnabled();
    });
    expect(screen.getByLabelText("Embedding Model")).toHaveValue("embed-3");
    expect(screen.queryByText("Settings saved")).not.toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/settings/runtime");
  });

  it("drops the pending state when a group is edited back to its baseline", async () => {
    setup();
    await openSettings("/settings/runtime");

    await userEvent.type(screen.getByLabelText("Embedding Model"), "-2");
    expect(screen.getByText("Unsaved changes")).toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("Embedding Model"));
    await userEvent.type(screen.getByLabelText("Embedding Model"), "embed-1");

    await waitFor(() => {
      expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();
    });
    expect(screen.getByTestId("settings-save")).toBeDisabled();
  });

  it("discards an unsaved draft on a fresh mount", async () => {
    const backend = setup();
    await openSettings("/settings/runtime");

    await userEvent.type(screen.getByLabelText("Embedding Model"), "-2");
    cleanup();

    await openSettings("/settings/runtime");

    expect(await screen.findByLabelText("Embedding Model")).toHaveValue("embed-1");
    expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(0);
  });

  it("runs the runtime probe with the unsaved draft and reports the outcome", async () => {
    const backend = setup({ fail: { testEmbedding: true } });
    await openSettings("/settings/runtime");

    await userEvent.clear(screen.getByLabelText("Embedding Base URL"));
    await userEvent.type(screen.getByLabelText("Embedding Base URL"), "http://embedding-2:8080/v1");
    await userEvent.click(screen.getByTestId("runtime-embedding-test"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/embedding/test")).toHaveLength(1);
    });
    const body = backend.callsFor("POST", "/v1/settings/runtime/embedding/test")[0].body as {
      base_url: string;
      api_key?: string;
    };
    expect(body.base_url).toBe("http://embedding-2:8080/v1");
    expect(body.api_key).toBeUndefined();
    expect(await screen.findByText("embedding refused")).toBeInTheDocument();
  });
});
