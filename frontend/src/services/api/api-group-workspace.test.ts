import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { unwrapFetchResponse, unwrapResponse } from "./api-core";
import { createGroupWorkspaceApi } from "./api-group-workspace";

const POST = vi.fn();
let capturedBody: BodyInit | null | undefined;

function createApi() {
  const authFetch = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
    capturedBody = init?.body;
    return new Response(JSON.stringify({ task_id: "task-1", item_ids: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
  return {
    authFetch,
    api: createGroupWorkspaceApi({
      authFetch,
      openapiClient: { POST, GET: vi.fn(), PUT: vi.fn(), DELETE: vi.fn(), PATCH: vi.fn() } as never,
      resolveApiUrl: (path) => path,
      unwrapFetchResponse,
      unwrapResponse,
    }),
  };
}

describe("group workspace source lifecycle upload", () => {
  beforeEach(() => {
    POST.mockReset();
    capturedBody = undefined;
    vi.stubGlobal("crypto", {
      subtle: { digest: vi.fn(async () => new ArrayBuffer(32)) },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("propagates the release-source toggle into prepare-upload and multipart metadata", async () => {
    POST.mockResolvedValue({ data: { upload_required: true }, response: { ok: true, status: 200 } });
    const { api } = createApi();

    await api.uploadGroupLibraryFiles("group-a", null, [new File(["hello"], "doc.txt", { type: "text/plain" })], {
      deleteSourceAfterProcessing: true,
    });

    expect(POST).toHaveBeenCalledWith(
      "/v1/groups/by-path/{group_path}/library/files/prepare-upload",
      expect.objectContaining({
        params: { path: { group_path: "group-a" } },
        body: expect.objectContaining({ delete_source_after_processing: true }),
      }),
    );
    const form = capturedBody as FormData;
    expect(String(form.get("metadata"))).toBe(JSON.stringify({
      delete_source_after_processing: true,
      metadata_json: {},
    }));
  });

  it("defaults to retaining the source", async () => {
    POST.mockResolvedValue({ data: { upload_required: true }, response: { ok: true, status: 200 } });
    const { api } = createApi();

    await api.uploadGroupLibraryFiles("group-a", null, [new File(["hello"], "doc.txt")]);

    const prepareCall = POST.mock.calls[0][1] as { body: Record<string, unknown> };
    expect(prepareCall.body.delete_source_after_processing).toBe(false);
    const form = capturedBody as FormData;
    expect(JSON.parse(String(form.get("metadata"))).delete_source_after_processing).toBe(false);
  });

  it("releases the source through the file-id endpoint", async () => {
    POST.mockResolvedValue({ data: { file_id: "f1", source_available: false }, response: { ok: true, status: 200 } });
    const { api } = createApi();

    await api.releaseGroupLibraryFileSource("group-a", "f1");

    expect(POST).toHaveBeenCalledWith(
      "/v1/groups/by-path/{group_path}/library/files/{file_id}/release-source",
      {
        params: { path: { group_path: "group-a", file_id: "f1" } },
        signal: undefined,
      },
    );
  });
});
