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

  it("switches the chunking budget to tokens and keeps the character budget", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/runtime");

    // A stored response that carries no token budget must not read as an
    // unsaved change.
    expect(await screen.findByLabelText("Max Characters")).toHaveValue(1200);
    expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();

    await userEvent.click(screen.getByLabelText("Budget Unit"));
    await userEvent.click(await screen.findByRole("option", { name: "Tokens" }));

    expect(await screen.findByLabelText("Max Tokens")).toHaveValue(1024);
    expect(screen.queryByLabelText("Max Characters")).not.toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("Max Tokens"));
    await userEvent.type(screen.getByLabelText("Max Tokens"), "512");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      chunking: {
        unit: string;
        tokenizer: string;
        tokenizer_sha256: string | null;
        max_chars: number;
        overlap_chars: number;
        max_tokens: number;
        overlap_tokens: number;
      };
    };
    // The request carries the generated contract exactly: the inactive
    // character budget is retained and the token budget is sent beside it.
    expect(body.chunking).toEqual({
      unit: "tokens",
      tokenizer: "cl100k_base",
      tokenizer_sha256: null,
      max_chars: 1200,
      overlap_chars: 200,
      max_tokens: 512,
      overlap_tokens: 128,
    });
  });

  it("keeps one tokenizer control and prepares the selected preset on demand", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/runtime");

    // The upper section owns no tokenizer of its own any more.
    expect(screen.queryByLabelText("Input tokenizer")).not.toBeInTheDocument();
    const tokenizer = await screen.findByLabelText("Tokenizer");
    expect(tokenizer).toBeInTheDocument();

    await userEvent.click(tokenizer);
    await userEvent.click(
      await screen.findByRole("option", { name: "Jina embeddings v5 text small retrieval" }),
    );

    // The license is visible before the preparation starts.
    expect(
      screen.getByText(/License cc-by-nc-4\.0/),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(
        backend.callsFor("POST", "/v1/settings/runtime/tokenizer-artifacts/provision"),
      ).toHaveLength(1);
    });
    expect(
      backend.callsFor("POST", "/v1/settings/runtime/tokenizer-artifacts/provision")[0].body,
    ).toEqual({ preset_id: "jina_v5_small_retrieval_r6856e76" });

    await waitFor(() => {
      expect(
        screen.getByTestId("runtime-tokenizer-preset-status-jina_v5_small_retrieval_r6856e76"),
      ).toHaveTextContent("verified and active");
    });

    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      chunking: { tokenizer: string; tokenizer_sha256: string | null };
      embedding: {
        input_policy: { tokenizer: string; tokenizer_sha256: string | null; add_special_tokens: boolean };
      };
    };
    // The request derives the input check from the lower selection.
    expect(body.chunking.tokenizer).toBe("hf_local");
    expect(body.chunking.tokenizer_sha256).toBe(
      "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    );
    expect(body.embedding.input_policy.tokenizer).toBe("hf_local");
    expect(body.embedding.input_policy.tokenizer_sha256).toBe(
      "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    );
    expect(body.embedding.input_policy.add_special_tokens).toBe(false);
  });

  it("keeps a built-in tokenizer free of a pinned artifact and of special tokens", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/runtime");

    await userEvent.click(await screen.findByLabelText("Tokenizer"));
    await userEvent.click(
      await screen.findByRole("option", { name: "Jina embeddings v5 text small retrieval" }),
    );
    await waitFor(() => {
      expect(
        backend.callsFor("POST", "/v1/settings/runtime/tokenizer-artifacts/provision"),
      ).toHaveLength(1);
    });

    await userEvent.click(await screen.findByLabelText("Tokenizer"));
    await userEvent.click(await screen.findByRole("option", { name: "o200k_base" }));

    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      chunking: { tokenizer: string; tokenizer_sha256: string | null };
      embedding: { input_policy: { tokenizer: string; tokenizer_sha256: string | null } };
    };
    expect(body.chunking.tokenizer).toBe("o200k_base");
    expect(body.chunking.tokenizer_sha256).toBeNull();
    expect(body.embedding.input_policy.tokenizer).toBe("o200k_base");
    expect(body.embedding.input_policy.tokenizer_sha256).toBeNull();
  });

  it("refuses the save while a selected preset fails to prepare and retries it", async () => {
    const backend = installFakeBackend({ fail: { tokenizerProvisionFails: 1 } });
    await openSettings("/settings/runtime");

    await userEvent.click(await screen.findByLabelText("Tokenizer"));
    await userEvent.click(
      await screen.findByRole("option", { name: "Jina embeddings v5 text small retrieval" }),
    );

    await waitFor(() => {
      expect(
        screen.getByTestId("runtime-tokenizer-preset-status-jina_v5_small_retrieval_r6856e76"),
      ).toHaveTextContent("could not be prepared");
    });

    // The failed preparation blocks the save.
    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(0);
    });

    await userEvent.click(
      screen.getByTestId("runtime-tokenizer-preset-prepare-jina_v5_small_retrieval_r6856e76"),
    );
    await waitFor(() => {
      expect(
        backend.callsFor("POST", "/v1/settings/runtime/tokenizer-artifacts/provision"),
      ).toHaveLength(2);
    });
    await waitFor(() => {
      expect(
        screen.getByTestId("runtime-tokenizer-preset-status-jina_v5_small_retrieval_r6856e76"),
      ).toHaveTextContent("verified and active");
    });

    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
  });

  it("blocks the save until the selected preset is prepared", async () => {
    const { RUNTIME_SETTINGS } = await import("./test-support");
    const backend = installFakeBackend({
      runtime: {
        ...RUNTIME_SETTINGS,
        chunking: {
          ...RUNTIME_SETTINGS.chunking,
          tokenizer: "hf_local",
          tokenizer_sha256:
            "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
        },
      },
    });
    await openSettings("/settings/runtime");

    // A persisted hash renders the preset even though this process has not
    // prepared it, and nothing is downloaded on render.
    expect(
      await screen.findByTestId("runtime-tokenizer-preset-status-jina_v5_small_retrieval_r6856e76"),
    ).toHaveTextContent("not prepared yet");
    expect(
      backend.callsFor("POST", "/v1/settings/runtime/tokenizer-artifacts/provision"),
    ).toHaveLength(0);

    await userEvent.clear(screen.getByLabelText("Max Characters"));
    await userEvent.type(screen.getByLabelText("Max Characters"), "1300");
    // The shared Save is disabled while the selected preset is not prepared.
    expect(screen.getByTestId("settings-save")).toBeDisabled();
    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(0);
    });

    // Preparing the preset releases the save without a restart.
    await userEvent.click(
      screen.getByTestId("runtime-tokenizer-preset-prepare-jina_v5_small_retrieval_r6856e76"),
    );
    await waitFor(() => {
      expect(
        screen.getByTestId("runtime-tokenizer-preset-status-jina_v5_small_retrieval_r6856e76"),
      ).toHaveTextContent("verified and active");
    });
    await waitFor(() => {
      expect(screen.getByTestId("settings-save")).toBeEnabled();
    });
    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
  });

  it("warns once when the stored input check differs from the lower selection", async () => {
    const { RUNTIME_SETTINGS } = await import("./test-support");
    const backend = installFakeBackend({
      runtime: {
        ...RUNTIME_SETTINGS,
        embedding: {
          ...RUNTIME_SETTINGS.embedding,
          input_policy: {
            ...RUNTIME_SETTINGS.embedding.input_policy!,
            tokenizer: "o200k_base",
          },
        },
      },
    });
    await openSettings("/settings/runtime");

    expect(
      await screen.findByTestId("runtime-chunking-input-policy-warning"),
    ).toBeInTheDocument();
    // A stored difference is not an unsaved change.
    expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("Max Characters"));
    await userEvent.type(screen.getByLabelText("Max Characters"), "1300");
    await userEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/runtime")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/runtime")[0].body as {
      embedding: { input_policy: { tokenizer: string } };
    };
    expect(body.embedding.input_policy.tokenizer).toBe("cl100k_base");

    // The saved envelope now matches the selection, so the warning is gone.
    await waitFor(() => {
      expect(
        screen.queryByTestId("runtime-chunking-input-policy-warning"),
      ).not.toBeInTheDocument();
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
      expect(
        backend.callsFor("GET", "/v1/settings/runtime/vector-index/rebuild").length,
      ).toBeGreaterThan(0);
    });
  });
});

describe("vector index rebuild control", () => {
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

  it("pauses the exact rebuild its status observed", async () => {
    const backend = installFakeBackend({
      rebuildStatus: {
        ...(await import("./test-support")).VECTOR_REBUILD_STATUS,
        state: "running",
        processed_chunks: 4,
        total_chunks: 10,
        can_pause: true,
        can_cancel: true,
      },
    });
    await openSettings("/settings/runtime");

    const pause = await screen.findByTestId("runtime-vector-rebuild-pause");
    await userEvent.click(pause);

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild/pause");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({ task_id: "task-vector-1" });
    });
    // A control is never a resubmission.
    expect(backend.callsFor("POST", "/v1/settings/runtime/vector-index/rebuild")).toHaveLength(0);
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

  it("shows the stored OCR preset and saves a selected one", async () => {
    const backend = installFakeBackend();
    await openSettings("/settings/docling");

    const preset = await screen.findByLabelText("OCR Preset");
    expect(preset).toHaveValue("rapidocr");

    await userEvent.clear(preset);
    await userEvent.type(preset, "custom.ocr-preset");
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(backend.callsFor("PUT", "/v1/settings/docling")).toHaveLength(1);
    });
    const body = backend.callsFor("PUT", "/v1/settings/docling")[0].body as {
      ocr_preset?: string;
    };
    expect(body.ocr_preset).toBe("custom.ocr-preset");
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
