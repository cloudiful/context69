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
});
