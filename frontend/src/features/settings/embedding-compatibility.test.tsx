import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "@/services/api";
import { installMockStorage } from "@/test-utils/storage";

import {
  installDomPolyfills,
  installFakeBackend,
  openSettings,
  resetAuth,
  resetPreferences,
  resetToasts,
  RUNTIME_SETTINGS,
  type FakeBackendOptions,
} from "./test-support";

function settingsNav() {
  return screen.getByRole("navigation", { name: "Settings" });
}

async function goToSection(
  router: { state: { location: { pathname: string } } },
  label: string,
  path: string,
) {
  await userEvent.click(within(settingsNav()).getByRole("link", { name: label }));
  const leave = screen.queryByRole("alertdialog");
  if (leave) {
    await userEvent.click(within(leave).getByRole("button", { name: "Leave" }));
  }
  await waitFor(() => {
    expect(router.state.location.pathname).toBe(path);
  });
}

function runtimePuts(backend: ReturnType<typeof installFakeBackend>) {
  return backend.callsFor("PUT", "/v1/settings/runtime");
}

function setup(options: FakeBackendOptions = {}) {
  return installFakeBackend(options);
}

describe("embedding compatibility confirmation", () => {
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

  it("writes nothing when a gateway change is not confirmed", async () => {
    const backend = setup();
    const router = await openSettings("/settings/search");

    await userEvent.clear(screen.getByLabelText("Rerank Candidates"));
    await userEvent.type(screen.getByLabelText("Rerank Candidates"), "55");
    await goToSection(router, "Embedding", "/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "alias-model");
    await userEvent.click(screen.getByTestId("settings-save"));

    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Keep existing vectors?")).toBeInTheDocument();
    expect(runtimePuts(backend)).toHaveLength(0);
    // The confirmation gates the whole save, so the pending search edit is not
    // written either.
    expect(backend.callsFor("PUT", "/v1/settings/search")).toHaveLength(0);

    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(runtimePuts(backend)).toHaveLength(0);
    expect(backend.callsFor("PUT", "/v1/settings/search")).toHaveLength(0);
    expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
  });

  it("submits the acknowledgement only for an accepted candidate and spends it", async () => {
    const backend = setup();
    await openSettings("/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "alias-model");
    await userEvent.click(screen.getByTestId("settings-save"));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Keep vectors and save" }));

    await waitFor(() => {
      expect(runtimePuts(backend)).toHaveLength(1);
    });
    const body = runtimePuts(backend)[0].body as {
      embedding: { model: string; confirm_vector_space_unchanged?: boolean };
    };
    expect(body.embedding.model).toBe("alias-model");
    expect(body.embedding.confirm_vector_space_unchanged).toBe(true);

    // A credential-only change keeps the live-safe path: no dialog, no flag.
    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeDisabled();
    });
    await userEvent.type(screen.getByLabelText("Embedding API Key"), "rotated-key");
    await userEvent.click(screen.getByTestId("settings-save"));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(runtimePuts(backend)).toHaveLength(2);
    });
    const credentialBody = runtimePuts(backend)[1].body as {
      embedding: { confirm_vector_space_unchanged?: boolean; api_key?: string };
    };
    expect(credentialBody.embedding.api_key).toBe("rotated-key");
    expect(credentialBody.embedding.confirm_vector_space_unchanged).toBeUndefined();
  });

  it("binds the confirmation to the candidate shown and re-prompts after a new edit", async () => {
    const backend = setup();
    await openSettings("/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "alias-one");
    await userEvent.click(screen.getByTestId("settings-save"));
    const firstDialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(firstDialog).getByRole("button", { name: "Cancel" }));

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "alias-two");
    await userEvent.click(screen.getByTestId("settings-save"));
    const secondDialog = await screen.findByRole("alertdialog");
    await userEvent.click(
      within(secondDialog).getByRole("button", { name: "Keep vectors and save" }),
    );

    await waitFor(() => {
      expect(runtimePuts(backend)).toHaveLength(1);
    });
    const body = runtimePuts(backend)[0].body as {
      embedding: { model: string; confirm_vector_space_unchanged?: boolean };
    };
    expect(body.embedding.model).toBe("alias-two");
    expect(body.embedding.confirm_vector_space_unchanged).toBe(true);

    // Success spends the confirmation: the next gateway edit asks again.
    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeDisabled();
    });
    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "alias-three");
    await userEvent.click(screen.getByTestId("settings-save"));
    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    expect(runtimePuts(backend)).toHaveLength(1);
  });

  it("never offers confirmation for a dimension change", async () => {
    const backend = setup();
    await openSettings("/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Dimensions"));
    await userEvent.type(screen.getByLabelText("Dimensions"), "1024");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(runtimePuts(backend)).toHaveLength(1);
    });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    const body = runtimePuts(backend)[0].body as {
      embedding: { dimensions: number; confirm_vector_space_unchanged?: boolean };
    };
    expect(body.embedding.dimensions).toBe(1024);
    expect(body.embedding.confirm_vector_space_unchanged).toBeUndefined();
  });

  it("reports a pending rebuild without starting one", async () => {
    const backend = setup({
      runtime: { ...RUNTIME_SETTINGS, requires_rebuild: true },
    });
    const router = await openSettings("/settings/vector-index");

    const status = await screen.findByTestId("runtime-requires-rebuild");
    expect(status).toHaveTextContent("Search and ingestion stay unavailable");
    expect(
      within(status).getByRole("button", { name: "Rebuild Vector Index" }),
    ).toBeInTheDocument();
    expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(0);

    // A save never clears or triggers the requirement by itself.
    await router.navigate("/settings/embedding");
    await userEvent.type(await screen.findByLabelText("Embedding API Key"), "rotated-key");
    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeDisabled();
    });
    expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(0);
    await router.navigate("/settings/vector-index");
    expect(await screen.findByTestId("runtime-requires-rebuild")).toBeInTheDocument();
  });

  it("keeps the rebuild a separate, explicitly confirmed full re-embed", async () => {
    const backend = setup({
      runtime: { ...RUNTIME_SETTINGS, requires_rebuild: true },
    });
    await openSettings("/settings/vector-index");

    await userEvent.click(await screen.findByTestId("runtime-requires-rebuild-action"));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/embedding cost/i)).toBeInTheDocument();
    expect(within(dialog).getByText(/not deleted or recreated/i)).toBeInTheDocument();
    expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(0);

    await userEvent.click(within(dialog).getByRole("button", { name: "Rebuild Vector Index" }));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(1);
    });
  });

  it("never saves or confirms from the connection test", async () => {
    const backend = setup();
    await openSettings("/settings/embedding");

    await userEvent.clear(screen.getByLabelText("Document model"));
    await userEvent.type(screen.getByLabelText("Document model"), "alias-model");
    await userEvent.clear(screen.getByLabelText("Query model"));
    await userEvent.type(screen.getByLabelText("Query model"), "invalid-query-alias");
    await userEvent.click(screen.getByTestId("runtime-embedding-test"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/embedding/test")).toHaveLength(1);
    });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(runtimePuts(backend)).toHaveLength(0);
    const body = backend.callsFor("POST", "/v1/settings/runtime/embedding/test")[0].body as {
      confirm_vector_space_unchanged?: boolean;
      model: string;
      query_model?: string;
    };
    expect(body.model).toBe("alias-model");
    expect(body.query_model).toBe("invalid-query-alias");
    expect(body.confirm_vector_space_unchanged).toBeUndefined();
  });

  it("discards a pending probe outcome after the query alias is edited", async () => {
    setup();
    await openSettings("/settings/embedding");

    let resolveProbe!: () => void;
    const pendingProbe = new Promise<void>((resolve) => {
      resolveProbe = resolve;
    });
    const probe = vi
      .spyOn(apiClient, "testEmbeddingConnection")
      .mockReturnValue(pendingProbe as never);

    await userEvent.click(screen.getByTestId("runtime-embedding-test"));
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(1));

    // The request the pending probe checks is no longer the observed draft.
    await userEvent.clear(screen.getByLabelText("Query model"));
    await userEvent.type(screen.getByLabelText("Query model"), "changed-alias");

    await act(async () => {
      resolveProbe();
      await pendingProbe;
    });

    // The old success must not be presented for the new request, and the form
    // must leave its testing state.
    await waitFor(() => {
      expect(screen.queryByText("Embedding connection succeeded")).not.toBeInTheDocument();
    });
    expect(screen.getByTestId("runtime-embedding-test")).toBeEnabled();
  });
});
