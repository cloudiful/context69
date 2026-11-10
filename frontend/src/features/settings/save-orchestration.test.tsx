import { cleanup, renderHook, screen, waitFor, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { restoreSession } from "@/lib/auth-store";
import { i18n } from "@/lib/i18n";
import { apiClient } from "@/services/api";
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
import { shouldRetryStatusPoll } from "./use-vector-rebuild";
import { useVectorRebuild } from "./use-vector-rebuild";
import { RUNTIME_SETTINGS, VECTOR_REBUILD_STATUS } from "./test-support";

const CODE_EMBEDDING = {
  enabled: true,
  base_url: "http://code-embedding:8080/v1",
  model: "code-embed-1",
  dimensions: 512,
  timeout_secs: 20,
  has_api_key: true,
};

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
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("retries a transient rebuild status failure a bounded number of times", () => {
    expect(shouldRetryStatusPoll(0)).toBe(true);
    expect(shouldRetryStatusPoll(3)).toBe(true);
    expect(shouldRetryStatusPoll(4)).toBe(false);
    expect(shouldRetryStatusPoll(5)).toBe(false);
  });

  it("recovers a transient rebuild poll failure and stops after an external cancel", async () => {
    installFakeBackend({});
    await restoreSession();
    const running = {
      ...VECTOR_REBUILD_STATUS,
      state: "running" as const,
      can_pause: true,
      can_cancel: true,
    };
    const cancelled = {
      ...VECTOR_REBUILD_STATUS,
      state: "cancelled" as const,
      can_pause: false,
      can_cancel: false,
    };
    const statusRead = vi
      .spyOn(apiClient, "getVectorIndexRebuildStatus")
      .mockRejectedValueOnce(new Error("status unavailable"))
      .mockResolvedValue(running);
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
    );
    const { result } = renderHook(() => useVectorRebuild(), { wrapper });

    await waitFor(() => expect(statusRead).toHaveBeenCalledTimes(1));
    expect(result.current.status).toBeNull();

    // The transient failure keeps the last known status and reschedules the
    // read instead of stopping the poll.
    await act(async () => {
      vi.advanceTimersByTime(1_500);
    });
    await waitFor(() => expect(result.current.status?.state).toBe("running"));

    // Another operator cancels the rebuild; the recovered poll observes it and
    // stops.
    statusRead.mockResolvedValue(cancelled);
    await act(async () => {
      vi.advanceTimersByTime(1_500);
    });
    await waitFor(() => expect(result.current.status?.state).toBe("cancelled"));
    const calls = statusRead.mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(statusRead.mock.calls.length).toBe(calls);
  });

  it("stops at the runtime admission step when the runtime save fails", async () => {
    const backend = installFakeBackend({ fail: { runtimePut: true } });
    const router = await openSettings("/settings/embedding");

    await userEvent.type(screen.getByLabelText("Document model"), "-2");
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
    const router = await openSettings("/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "embed-2");

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
    const router = await openSettings("/settings/embedding");
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

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "embed-2");
    await userEvent.click(screen.getByTestId("settings-save"));
    await acceptCompatibility();

    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeDisabled();
    });

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "embed-3");

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
    expect(screen.getByLabelText("Document model")).toHaveValue("embed-3");
    expect(screen.queryByText("Settings saved")).not.toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/settings/embedding");
  });

  it("drops the pending state when a group is edited back to its baseline", async () => {
    setup();
    await openSettings("/settings/embedding");

    await userEvent.type(screen.getByLabelText("Document model"), "-2");
    expect(screen.getByText("Unsaved changes")).toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "embed-1");

    await waitFor(() => {
      expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();
    });
    expect(screen.getByTestId("settings-save")).toBeDisabled();
  });

  it("discards an unsaved draft on a fresh mount", async () => {
    const backend = setup();
    await openSettings("/settings/embedding");

    await userEvent.type(screen.getByLabelText("Document model"), "-2");
    cleanup();

    await openSettings("/settings/embedding");

    expect(await screen.findByLabelText("Document model")).toHaveValue("embed-1");
    expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(0);
  });

  it("runs the runtime probe with the unsaved draft and reports the outcome", async () => {
    const backend = setup({ fail: { testEmbedding: true } });
    await openSettings("/settings/embedding");

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

  it("omits an untouched code block so an ordinary save preserves it", async () => {
    const backend = installFakeBackend({
      runtime: { ...RUNTIME_SETTINGS, code_embedding: CODE_EMBEDDING },
    });
    await openSettings("/settings/embedding");

    await userEvent.type(screen.getByLabelText("Embedding API Key"), "document-key");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      code_embedding?: unknown;
    };
    expect(body.code_embedding).toBeUndefined();
    expect(backend.state.runtime.code_embedding?.model).toBe("code-embed-1");
    expect(backend.state.runtime.code_embedding?.has_api_key).toBe(true);
  });

  it("probes the code model with its own purpose and endpoint", async () => {
    const backend = installFakeBackend({
      runtime: { ...RUNTIME_SETTINGS, code_embedding: CODE_EMBEDDING },
    });
    await openSettings("/settings/embedding");

    await userEvent.click(screen.getByTestId("runtime-code-embedding-test"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/embedding/test")).toHaveLength(1);
    });
    const body = backend.callsFor("POST", "/v1/settings/runtime/embedding/test")[0].body as {
      purpose?: string;
      base_url: string;
      api_key?: string;
    };
    expect(body.purpose).toBe("code");
    expect(body.base_url).toBe("http://code-embedding:8080/v1");
    expect(body.api_key).toBeUndefined();
  });

  it("sends an edited code block without touching document identity", async () => {
    const backend = installFakeBackend({
      runtime: { ...RUNTIME_SETTINGS, code_embedding: CODE_EMBEDDING },
    });
    await openSettings("/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Code model"));
    await userEvent.type(screen.getByLabelText("Code model"), "code-embed-2");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      embedding: { model: string; confirm_vector_space_unchanged?: boolean };
      code_embedding?: { enabled: boolean; model: string; api_key?: { op: string } };
    };
    expect(body.code_embedding?.enabled).toBe(true);
    expect(body.code_embedding?.model).toBe("code-embed-2");
    expect(body.code_embedding?.api_key?.op).toBe("keep");
    expect(body.embedding.model).toBe("embed-1");
    expect(body.embedding.confirm_vector_space_unchanged).toBeUndefined();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(backend.state.runtime.code_embedding?.model).toBe("code-embed-2");
  });
});
