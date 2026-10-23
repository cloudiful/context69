import { screen, waitFor, within } from "@testing-library/react";
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
} from "./test-support";

describe("search settings", () => {
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

  it("shows the real initial load reason and recovers through Retry", async () => {
    installFakeBackend({ runtimeGetFailReads: 1 });
    await openSettings("/settings/search");
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("runtime unavailable");
    expect(alert).not.toHaveTextContent("Loading settings");
    await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByLabelText("Rerank Candidates")).toHaveValue(40);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("saves the rerank fields and a write-only key", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/search");

    expect(await screen.findByLabelText("Rerank Candidates")).toHaveValue(40);
    await userEvent.clear(screen.getByLabelText("Rerank Candidates"));
    await userEvent.type(screen.getByLabelText("Rerank Candidates"), "55");
    await userEvent.type(screen.getByLabelText("Rerank API Key"), "sk-test-secret");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/search")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/search")[0].body as {
      mode: string;
      candidate_limit: number;
      rerank_enabled: boolean;
      vector_weight: number;
      keyword_weight: number;
      api_key: { op: string; value?: string };
    };
    expect(body.mode).toBe("hybrid");
    expect(body.candidate_limit).toBe(55);
    expect(body.rerank_enabled).toBe(true);
    expect(body.vector_weight).toBeCloseTo(0.55);
    expect(body.keyword_weight).toBeCloseTo(0.35);
    expect(body.api_key).toEqual({ op: "set", value: "sk-test-secret" });

    await waitFor(() => {
      expect(screen.getByLabelText("Rerank API Key")).toHaveValue("");
    });
    expect(JSON.stringify({ ...window.localStorage })).not.toContain("sk-test-secret");
  });
});

describe("runtime settings", () => {
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

  it("probes the unsaved Qdrant URL and disables the probe when it is blank", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/runtime");

    const probe = await screen.findByTestId("runtime-qdrant-test");
    expect(probe).toBeEnabled();

    await userEvent.clear(screen.getByLabelText("Qdrant URL"));
    expect(probe).toBeDisabled();

    await userEvent.type(screen.getByLabelText("Qdrant URL"), "http://qdrant-2:6333");
    await userEvent.click(probe);

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/qdrant/test")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/settings/runtime/qdrant/test")[0].body).toEqual({
      url: "http://qdrant-2:6333",
    });
  });

  it("probes valkey, S3 and embedding with the draft values", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/runtime");

    await userEvent.click(screen.getByTestId("runtime-valkey-test"));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/valkey/test")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/settings/runtime/valkey/test")[0].body).toEqual({
      valkey_url: "redis://valkey:6379/0",
    });

    await userEvent.click(screen.getByRole("switch", { name: "Use S3 object storage" }));
    await userEvent.type(await screen.findByLabelText("S3 Endpoint"), "https://s3.example.test");
    await userEvent.type(screen.getByLabelText("S3 Bucket"), "context69");
    await userEvent.click(screen.getByTestId("runtime-file-library-s3-test"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/s3/test")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/settings/runtime/s3/test")[0].body).toMatchObject({
      endpoint: "https://s3.example.test",
      bucket: "context69",
    });

    await userEvent.click(screen.getByTestId("runtime-embedding-test"));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/embedding/test")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/settings/runtime/embedding/test")[0].body).toMatchObject({
      base_url: "http://embedding:8080/v1",
      model: "embed-1",
      dimensions: 768,
    });
  });

  it("requires a confirmation before submitting a vector index rebuild", async () => {
    const backend = installFakeBackend({
      vectorTask: { ...(await import("./test-support")).VECTOR_TASK, status: "succeeded" },
    });
    await openSettings("/settings/runtime");

    expect(await screen.findByText("Rebuilt 10 chunks")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("runtime-vector-rebuild"));
    const dialog = await screen.findByRole("alertdialog");
    expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(0);

    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(0);

    await userEvent.click(screen.getByTestId("runtime-vector-rebuild"));
    const confirmDialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(confirmDialog).getByRole("button", { name: "Rebuild Vector Index" }));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(1);
    });
    await waitFor(() => {
      expect(backend.callsFor("GET", "/v1/tasks/{task_id}").length).toBeGreaterThan(0);
    });
  });
});

describe("docling settings", () => {
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

  it("sends the connection probe with the unsaved draft", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/docling");

    await userEvent.clear(screen.getByLabelText("Base URL"));
    await userEvent.type(screen.getByLabelText("Base URL"), "http://docling-2:5001");
    await userEvent.click(screen.getByTestId("docling-connection-test"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/settings/docling/test")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/settings/docling/test")[0].body).toMatchObject({
      base_url: "http://docling-2:5001",
      timeout_secs: 120,
    });
  });

  it("projects the selected VLM mode onto the saved payload", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/docling");

    await userEvent.click(screen.getByLabelText("VLM Mode"));
    await userEvent.click(await screen.findByRole("option", { name: "Picture Description Preset" }));
    await userEvent.type(await screen.findByLabelText("Picture Description Preset"), "granite_vision");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/docling")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/docling")[0].body as {
      vlm: Record<string, unknown>;
    };
    expect(body.vlm.picture_description_preset).toBe("granite_vision");
    expect(body.vlm.vlm_pipeline_model).toBeUndefined();
    expect(body.vlm.api_key).toBeUndefined();
  });

  it("lists remote tasks, pages forward and wakes stopped-task cleanup", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/docling");

    expect(await screen.findByText("remote-1")).toBeInTheDocument();
    expect(screen.getByText("convert")).toBeInTheDocument();
    expect(screen.getByText("cleanup-1")).toBeInTheDocument();
    const cleanupReads = backend.callsFor("GET", "/v1/admin/docling/cleanup");
    expect(cleanupReads).toHaveLength(1);
    expect((cleanupReads[0].params as { query: Record<string, unknown> }).query).toMatchObject({
      page: 1,
      page_size: 20,
    });

    await userEvent.click(screen.getByTestId("docling-remote-next"));
    await waitFor(() => {
      expect(backend.callsFor("GET", "/v1/admin/docling/tasks").length).toBeGreaterThan(1);
    });
    const lastList = backend.callsFor("GET", "/v1/admin/docling/tasks").at(-1)!;
    expect((lastList.params as { query: { offset: number } }).query.offset).toBe(20);

    await userEvent.click(screen.getByRole("button", { name: "Clean up stopped tasks" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/not only this page/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Running work and started conversions are not stopped/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Clean up stopped tasks" }));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/admin/docling/cleanup")).toHaveLength(1);
    });
    expect(await screen.findByTestId("docling-cleanup-woken")).toHaveTextContent("Woken 2");
  });
});
