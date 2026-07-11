import { afterEach, describe, expect, it, vi } from "vitest";

const { GET, POST, PUT, DELETE, PATCH } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PUT: vi.fn(),
  DELETE: vi.fn(),
  PATCH: vi.fn(),
}));

vi.mock("./openapi-client", () => ({
  openapiClient: {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
  },
}));

import { ApiError, apiClient } from "./api";

describe("apiClient", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    GET.mockReset();
    POST.mockReset();
    PUT.mockReset();
    DELETE.mockReset();
    PATCH.mockReset();
  });

  it("posts search payloads and returns typed results", async () => {
    POST.mockResolvedValue({
      data: {
        query: "policy",
        hits: [],
      },
      response: {
        ok: true,
        status: 200,
      },
    });

    const result = await apiClient.search({
      query: "policy",
      limit: 5,
      source_key: "gov_documents",
    });

    expect(result.query).toBe("policy");
    expect(POST).toHaveBeenCalledWith("/v1/search", {
      body: {
        query: "policy",
        limit: 5,
        source_key: "gov_documents",
      },
      signal: undefined,
    });
  });

  it("maps api errors with status code", async () => {
    GET.mockResolvedValue({
      error: {
        error: "document not found",
      },
      response: {
        ok: false,
        status: 404,
      },
    });

    await expect(apiClient.getDocument(42)).rejects.toEqual(
      expect.objectContaining<ApiError>({
        name: "ApiError",
        message: "document not found",
        status: 404,
      }),
    );
  });

  it("handles no-content responses for delete", async () => {
    DELETE.mockResolvedValue({
      data: undefined,
      response: {
        ok: true,
        status: 204,
      },
    });

    await expect(apiClient.deleteSource("gov_documents")).resolves.toBeUndefined();
    expect(DELETE).toHaveBeenCalledWith("/v1/sources/{source_key}", {
      params: {
        path: {
          source_key: "gov_documents",
        },
      },
      signal: undefined,
    });
  });

  it("tests a Valkey connection without saving runtime settings", async () => {
    POST.mockResolvedValue({
      data: undefined,
      response: { ok: true, status: 204 },
    });

    await expect(apiClient.testValkeyConnection({
      valkey_url: "redis://valkey:6379/0",
    })).resolves.toBeUndefined();
    expect(POST).toHaveBeenCalledWith("/v1/settings/runtime/valkey/test", {
      body: { valkey_url: "redis://valkey:6379/0" },
      signal: undefined,
    });
  });

  it("gets and updates docling settings", async () => {
    GET.mockResolvedValueOnce({
      data: {
        configured: false,
        source: "unconfigured",
        connection: {
          base_url: null,
          timeout_secs: 120,
          poll_interval_secs: 2,
        },
        vlm: {
          openai_base_url: null,
          has_api_key: false,
        },
      },
      response: {
        ok: true,
        status: 200,
      },
    });
    PUT.mockResolvedValueOnce({
      data: {
        configured: true,
        source: "database",
        connection: {
          base_url: "http://docling:5001",
          timeout_secs: 180,
          poll_interval_secs: 3,
        },
        vlm: {
          openai_base_url: "https://openrouter.ai/api/v1",
          has_api_key: true,
          vlm_pipeline_model: "gemini-3-flash",
          picture_description_model: "gpt-4o-mini",
          code_formula_model: "gpt-4o-mini",
        },
      },
      response: {
        ok: true,
        status: 200,
      },
    });

    await expect(apiClient.getDoclingSettings()).resolves.toEqual(
      expect.objectContaining({
        source: "unconfigured",
      }),
    );
    await expect(
      apiClient.updateDoclingSettings({
        connection: {
          base_url: "http://docling:5001",
          timeout_secs: 180,
          poll_interval_secs: 3,
        },
        vlm: {
          openai_base_url: "https://openrouter.ai/api/v1",
          api_key: "secret",
          vlm_pipeline_model: "gemini-3-flash",
          picture_description_model: "gpt-4o-mini",
          code_formula_model: "gpt-4o-mini",
        },
      }),
    ).resolves.toEqual(
      expect.objectContaining({
        source: "database",
      }),
    );

    expect(GET).toHaveBeenCalledWith("/v1/settings/docling", {
      signal: undefined,
    });
    expect(PUT).toHaveBeenCalledWith("/v1/settings/docling", {
      body: {
        connection: {
          base_url: "http://docling:5001",
          timeout_secs: 180,
          poll_interval_secs: 3,
        },
        vlm: {
          openai_base_url: "https://openrouter.ai/api/v1",
          api_key: "secret",
          vlm_pipeline_model: "gemini-3-flash",
          picture_description_model: "gpt-4o-mini",
          code_formula_model: "gpt-4o-mini",
        },
      },
      signal: undefined,
    });
  });

  it("lists, creates, and revokes personal access tokens", async () => {
    GET.mockResolvedValueOnce({
      data: [
        {
          token_id: "00000000-0000-0000-0000-000000000001",
          name: "CLI",
          display_prefix: "ctx_pat_abcd",
          scopes: ["search", "library"],
          expires_at: "2026-12-31T00:00:00Z",
          last_used_at: null,
          revoked_at: null,
          created_at: "2026-06-01T00:00:00Z",
          updated_at: "2026-06-01T00:00:00Z",
        },
      ],
      response: {
        ok: true,
        status: 200,
      },
    });
    POST.mockResolvedValueOnce({
      data: {
        access_token: "ctx_pat_secret",
        token: {
          token_id: "00000000-0000-0000-0000-000000000001",
          name: "CLI",
          display_prefix: "ctx_pat_abcd",
          scopes: ["search"],
          expires_at: "2026-12-31T00:00:00Z",
          last_used_at: null,
          revoked_at: null,
          created_at: "2026-06-01T00:00:00Z",
          updated_at: "2026-06-01T00:00:00Z",
        },
      },
      response: {
        ok: true,
        status: 200,
      },
    });
    DELETE.mockResolvedValueOnce({
      data: undefined,
      response: {
        ok: true,
        status: 204,
      },
    });

    await expect(apiClient.listPersonalAccessTokens()).resolves.toHaveLength(1);
    await expect(apiClient.createPersonalAccessToken({
      name: "CLI",
      scopes: ["search"],
      expires_in_days: 30,
    })).resolves.toEqual(expect.objectContaining({
      access_token: "ctx_pat_secret",
    }));
    await expect(apiClient.revokePersonalAccessToken("00000000-0000-0000-0000-000000000001")).resolves.toBeUndefined();

    expect(GET).toHaveBeenCalledWith("/v1/auth/personal-access-tokens", {
      signal: undefined,
    });
    expect(POST).toHaveBeenCalledWith("/v1/auth/personal-access-tokens", {
      body: {
        name: "CLI",
        scopes: ["search"],
        expires_in_days: 30,
      },
      signal: undefined,
    });
    expect(DELETE).toHaveBeenCalledWith("/v1/auth/personal-access-tokens/{token_id}", {
      params: {
        path: {
          token_id: "00000000-0000-0000-0000-000000000001",
        },
      },
      signal: undefined,
    });
  });
});
