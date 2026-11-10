import { webcrypto } from "node:crypto";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import { apiClient } from "@/services/api";
import { openapiClient } from "@/services/openapi-client";

import type { PrepareLibraryUploadResponse, TaskRef } from "@/services/api";

interface PrepareCall {
  body: {
    folder_id?: string | null;
    filename: string;
    media_type: string;
    size_bytes: number;
    sha256: string;
    options?: { source_policy?: string; metadata?: { metadata_json?: unknown } };
  };
}

const uploadedTask: TaskRef = { task_id: "task-upload" };
const reusedFile = {
  file_id: "11111111-1111-4111-8111-111111111111",
  group_key: "group",
  group_path: "stock/alpha",
  visibility: "private" as const,
  folder_id: null,
  filename: "existing.txt",
  media_type: "text/plain",
  size_bytes: 4,
  ingest_status: "succeeded" as const,
  error_message: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  ingested_at: "2026-01-01T00:00:00Z",
};

async function digestOf(content: string): Promise<string> {
  const digest = await webcrypto.subtle.digest("SHA-256", new TextEncoder().encode(content));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

describe("library upload protocol", () => {
  beforeAll(() => {
    vi.stubGlobal("crypto", webcrypto);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function spyPrepare(resultFor: (filename: string) => PrepareLibraryUploadResponse) {
    const calls: PrepareCall[] = [];
    vi.spyOn(openapiClient, "POST").mockImplementation(((path: string, init?: { body?: PrepareCall["body"] }) => {
      if (path === "/v1/groups/by-path/{group_path}/library/files/prepare-upload") {
        calls.push({ body: init?.body as PrepareCall["body"] });
        const filename = init?.body?.filename ?? "";
        return Promise.resolve({ data: resultFor(filename), response: { ok: true, status: 200 } });
      }
      return Promise.resolve({ error: { message: "unmocked" }, response: { ok: false, status: 404 } });
    }) as never);
    return calls;
  }

  it("hashes every file and reuses non-upload results without a multipart request", async () => {
    const calls = spyPrepare(() => ({
      upload_required: false,
      file: reusedFile,
      task: { task_id: "task-reused" },
    }));
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);

    const response = await apiClient.uploadGroupLibraryFiles(
      "stock/alpha",
      null,
      [new File(["same"], "existing.txt", { type: "text/plain" })],
    );

    expect(calls).toHaveLength(1);
    expect(calls[0].body.sha256).toBe(await digestOf("same"));
    expect(calls[0].body.options?.source_policy).toBe("retain");
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(response.files.map((file) => file.file_id)).toEqual([reusedFile.file_id]);
    expect(response.tasks.map((task) => task.task_id)).toEqual(["task-reused"]);
  });

  it("sends one multipart request per pending file", async () => {
    spyPrepare((filename) => filename === "existing.txt"
      ? { upload_required: false, file: reusedFile, task: { task_id: "task-reused" } }
      : { upload_required: true, file: null, task: null });
    const fetchSpy = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response(JSON.stringify(uploadedTask), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }));
    vi.stubGlobal("fetch", fetchSpy);

    const response = await apiClient.uploadGroupLibraryFiles(
      "stock/alpha",
      "22222222-2222-4222-8222-222222222222",
      [
        new File(["fresh"], "fresh.txt", { type: "text/plain" }),
        new File(["second"], "second.txt", { type: "text/plain" }),
        new File(["old"], "existing.txt", { type: "text/plain" }),
      ],
    );

    expect(fetchSpy).toHaveBeenCalledTimes(2);
    const sent = fetchSpy.mock.calls.map(([url, init]) => {
      expect(url).toContain("/v1/groups/by-path/stock%2Falpha/library/files/upload");
      const form = (init as RequestInit).body as FormData;
      expect(form.get("folder_id")).toBe("22222222-2222-4222-8222-222222222222");
      expect(JSON.parse(String(form.get("metadata")))).toEqual({
        metadata: { metadata_json: {} },
        source_policy: "retain",
      });
      expect(form.getAll("files")).toHaveLength(1);
      return {
        name: (form.getAll("files") as File[])[0].name,
        sha256: form.get("sha256"),
      };
    });
    expect(sent).toEqual([
      { name: "fresh.txt", sha256: await digestOf("fresh") },
      { name: "second.txt", sha256: await digestOf("second") },
    ]);
    expect(response.tasks.map((task) => task.task_id)).toEqual([
      "task-reused",
      "task-upload",
      "task-upload",
    ]);
  });

  it("keeps accepted references when a later file fails to upload", async () => {
    spyPrepare(() => ({ upload_required: true, file: null, task: null }));
    let calls = 0;
    const fetchSpy = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => {
      calls += 1;
      if (calls === 1) {
        return new Response(JSON.stringify(uploadedTask), { status: 200 });
      }
      return new Response(JSON.stringify({ code: "storage", message: "storage refused" }), {
        status: 502,
        headers: { "Content-Type": "application/json" },
      });
    });
    vi.stubGlobal("fetch", fetchSpy);

    const error = await apiClient.uploadGroupLibraryFiles(
      "stock/alpha",
      null,
      [
        new File(["a"], "a.txt", { type: "text/plain" }),
        new File(["b"], "b.txt", { type: "text/plain" }),
      ],
    ).then(
      () => null,
      (failure: unknown) => failure as { name: string; tasks: TaskRef[]; files: unknown[] },
    );

    expect(fetchSpy).toHaveBeenCalledTimes(2);
    expect(error?.name).toBe("PartialUploadFailure");
    expect(error?.tasks.map((task) => task.task_id)).toEqual(["task-upload"]);
    expect(error?.files).toEqual([]);
  });

  it("propagates the release-after-processing policy into prepare and upload", async () => {
    const calls = spyPrepare(() => ({ upload_required: true, file: null, task: null }));
    const fetchSpy = vi.fn(async () => new Response(JSON.stringify(uploadedTask), { status: 200 }));
    vi.stubGlobal("fetch", fetchSpy);

    await apiClient.uploadGroupLibraryFiles(
      "stock/alpha",
      null,
      [new File(["one"], "one.txt", { type: "text/plain" })],
      { deleteSourceAfterProcessing: true },
    );

    expect(calls[0].body.options?.source_policy).toBe("release_after_processing");
    const form = (fetchSpy.mock.calls[0] as unknown as [string, RequestInit])[1].body as FormData;
    expect(JSON.parse(String(form.get("metadata"))).source_policy).toBe("release_after_processing");
  });

  it("keeps every settled reuse file and task when a prepare call fails before multipart", async () => {
    let index = 0;
    vi.spyOn(openapiClient, "POST").mockImplementation((() => {
      index += 1;
      if (index === 1) {
        return Promise.resolve({
          data: { upload_required: false, file: reusedFile, task: { task_id: "task-reused" } },
          response: { ok: true, status: 200 },
        });
      }
      if (index === 2) {
        return Promise.resolve({ error: { message: "prepare rejected" }, response: { ok: false, status: 422 } });
      }
      return Promise.resolve({ data: { upload_required: true, file: null, task: null }, response: { ok: true, status: 200 } });
    }) as never);
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);

    const error = await apiClient.uploadGroupLibraryFiles(
      "stock/alpha",
      null,
      [
        new File(["old"], "existing.txt", { type: "text/plain" }),
        new File(["broken"], "broken.txt", { type: "text/plain" }),
        new File(["fresh"], "fresh.txt", { type: "text/plain" }),
      ],
    ).then(
      () => null,
      (failure: unknown) => failure as { name: string; tasks: TaskRef[]; files: { file_id: string }[] },
    );

    expect(fetchSpy).not.toHaveBeenCalled();
    expect(error?.name).toBe("PartialUploadFailure");
    expect(error?.files.map((file) => file.file_id)).toEqual([reusedFile.file_id]);
    expect(error?.tasks.map((task) => task.task_id)).toEqual(["task-reused"]);
  });

  it("fails the whole batch before upload when one prepare call fails", async () => {
    let index = 0;
    vi.spyOn(openapiClient, "POST").mockImplementation((() => {
      index += 1;
      if (index === 1) {
        return Promise.resolve({ data: { upload_required: true, file: null, task: null }, response: { ok: true, status: 200 } });
      }
      return Promise.resolve({ error: { message: "prepare rejected" }, response: { ok: false, status: 422 } });
    }) as never);
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);

    await expect(apiClient.uploadGroupLibraryFiles(
      "stock/alpha",
      null,
      [
        new File(["a"], "a.txt", { type: "text/plain" }),
        new File(["b"], "b.txt", { type: "text/plain" }),
      ],
    )).rejects.toThrow("prepare rejected");
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});
