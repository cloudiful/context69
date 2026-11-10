import { describe, expect, it, vi } from "vitest";

import { unwrapResponse } from "./api-core";
import { createSettingsApi } from "./api-settings";

function createApi(get: ReturnType<typeof vi.fn>) {
  return createSettingsApi({
    openapiClient: { GET: get } as never,
    unwrapResponse,
  });
}

describe("settings api docling remote tasks", () => {
  it("wakes cleanup without caller-supplied remote identities", async () => {
    const post = vi.fn().mockResolvedValue({ data: { woken: 2 }, response: { ok: true, status: 200 } });
    const api = createSettingsApi({ openapiClient: { POST: post } as never, unwrapResponse });
    expect(await api.wakeDoclingCleanup()).toEqual({ woken: 2 });
    expect(post).toHaveBeenCalledWith("/v1/admin/docling/cleanup", { signal: undefined });
  });
  it("lists one bounded page with the saved-setting limit and offset", async () => {
    const data = {
      items: [
        {
          task_id: "task-1",
          task_type: "convert",
          task_status: "started",
          created_at: "2026-10-01T00:00:00Z",
        },
      ],
      has_more: true,
    };
    const get = vi.fn().mockResolvedValue({ data, response: { ok: true, status: 200 } });
    const api = createApi(get);

    const result = await api.listDoclingRemoteTasks({ limit: 20, offset: 20 });

    expect(get).toHaveBeenCalledWith("/v1/admin/docling/tasks", {
      params: { query: { limit: 20, offset: 20 } },
      signal: undefined,
    });
    expect(result).toBe(data);
  });

  it("forwards an abort signal so a stale list request can be cancelled", async () => {
    const controller = new AbortController();
    const get = vi.fn().mockResolvedValue({
      data: { items: [], has_more: false },
      response: { ok: true, status: 200 },
    });
    const api = createApi(get);

    await api.listDoclingRemoteTasks({ limit: 20, offset: 0 }, { signal: controller.signal });

    expect(get).toHaveBeenCalledWith(
      "/v1/admin/docling/tasks",
      expect.objectContaining({ signal: controller.signal }),
    );
  });

  it("pages and filters local cleanup rows on the server", async () => {
    const data = {
      items: [],
      pagination: { page: 2, page_size: 50, total: 120, total_pages: 3 },
    };
    const get = vi.fn().mockResolvedValue({ data, response: { ok: true, status: 200 } });
    const api = createApi(get);

    const result = await api.listDoclingCleanup({
      page: 2,
      pageSize: 50,
      status: "done",
      outcome: "success",
      remoteTaskId: "  cleanup-9  ",
    });

    expect(get).toHaveBeenCalledWith("/v1/admin/docling/cleanup", {
      params: {
        query: {
          page: 2,
          page_size: 50,
          status: "done",
          outcome: "success",
          remote_task_id: "cleanup-9",
        },
      },
      signal: undefined,
    });
    expect(result).toBe(data);
  });

  it("omits blank cleanup filters and forwards the abort signal", async () => {
    const controller = new AbortController();
    const get = vi.fn().mockResolvedValue({
      data: { items: [], pagination: { page: 1, page_size: 20, total: 0, total_pages: 1 } },
      response: { ok: true, status: 200 },
    });
    const api = createApi(get);

    await api.listDoclingCleanup({
      page: 1,
      pageSize: 20,
      status: null,
      outcome: null,
      remoteTaskId: "   ",
    }, { signal: controller.signal });

    expect(get).toHaveBeenCalledWith("/v1/admin/docling/cleanup", {
      params: {
        query: {
          page: 1,
          page_size: 20,
          status: undefined,
          outcome: undefined,
          remote_task_id: undefined,
        },
      },
      signal: controller.signal,
    });
  });
});

describe("settings api llm provider probe", () => {
  it("posts the unsaved draft to the llm test path without mutation", async () => {
    const post = vi.fn().mockResolvedValue({ data: { outcome: "success" }, response: { ok: true, status: 200 } });
    const api = createSettingsApi({ openapiClient: { POST: post } as never, unwrapResponse });
    const payload = {
      endpoint: "https://api.openai.com/v1",
      model: "gpt-4o-mini",
      llm_api_kind: "openai_responses",
    };

    await api.testLlmProviderConnection(payload);

    expect(post).toHaveBeenCalledWith("/v1/settings/translation/providers/llm/test", {
      body: payload,
      signal: undefined,
    });
  });

  it("forwards an abort signal so a stale probe can be cancelled", async () => {
    const controller = new AbortController();
    const post = vi.fn().mockResolvedValue({ data: { outcome: "success" }, response: { ok: true, status: 200 } });
    const api = createSettingsApi({ openapiClient: { POST: post } as never, unwrapResponse });

    await api.testLlmProviderConnection(
      { endpoint: "https://api.openai.com/v1", model: "gpt-4o-mini", llm_api_kind: "openai_responses" },
      { signal: controller.signal },
    );

    expect(post).toHaveBeenCalledWith(
      "/v1/settings/translation/providers/llm/test",
      expect.objectContaining({ signal: controller.signal }),
    );
  });
});

describe("settings api vector index rebuild control", () => {
  it("reads the durable status without a body", async () => {
    const status = { state: "running", task_id: "task-vector-1" };
    const get = vi.fn().mockResolvedValue({ data: status, response: { ok: true, status: 200 } });
    const api = createApi(get);

    expect(await api.getVectorIndexRebuildStatus()).toBe(status);
    expect(get).toHaveBeenCalledWith("/v1/settings/runtime/vector-index/rebuild", {
      signal: undefined,
    });
  });

  it("carries the observed task id on every control", async () => {
    const post = vi.fn().mockResolvedValue({
      data: { state: "pausing", task_id: "task-vector-1" },
      response: { ok: true, status: 200 },
    });
    const api = createSettingsApi({ openapiClient: { POST: post } as never, unwrapResponse });

    await api.pauseVectorIndexRebuild("task-vector-1");
    await api.resumeVectorIndexRebuild("task-vector-1");
    await api.cancelVectorIndexRebuild("task-vector-1");

    for (const action of ["pause", "resume", "cancel"]) {
      expect(post).toHaveBeenCalledWith(
        `/v1/settings/runtime/vector-index/rebuild/${action}`,
        { body: { task_id: "task-vector-1" }, signal: undefined },
      );
    }
  });
});

describe("settings api tokenizer preset preparation", () => {
  it("sends only the known preset identifier", async () => {
    const response = {
      preset_id: "embeddinggemma2_r_f9972ac",
      sha256: "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
      size_bytes: 32170510,
      status: "provisioned",
      ready: true,
      license: "apache-2.0",
    };
    const post = vi.fn().mockResolvedValue({ data: response, response: { ok: true, status: 200 } });
    const api = createSettingsApi({ openapiClient: { POST: post } as never, unwrapResponse });

    expect(
      await api.provisionTokenizerPreset({ preset_id: "embeddinggemma2_r_f9972ac" }),
    ).toBe(response);
    expect(post).toHaveBeenCalledWith(
      "/v1/settings/runtime/tokenizer-artifacts/provision",
      {
        body: { preset_id: "embeddinggemma2_r_f9972ac" },
        signal: undefined,
      },
    );
  });
});
