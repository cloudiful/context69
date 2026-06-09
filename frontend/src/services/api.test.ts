import { afterEach, describe, expect, it, vi } from "vitest";

const { GET, POST, PUT, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PUT: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock("./openapi-client", () => ({
  openapiClient: {
    GET,
    POST,
    PUT,
    DELETE,
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
        conversion: {},
        ocr: {
          do_ocr: true,
          force_ocr: false,
          ocr_lang: [],
        },
        enrichment: {},
        vlm: {
          provider_account_key: null,
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
        conversion: {
          pdf_backend: "dlparse_v2",
        },
        ocr: {
          do_ocr: true,
          force_ocr: false,
          ocr_engine: "rapidocr",
          ocr_lang: ["en"],
        },
        enrichment: {
          do_picture_description: true,
        },
        vlm: {
          provider_account_key: "openrouter-default",
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
        conversion: {
          pdf_backend: "dlparse_v2",
        },
        ocr: {
          do_ocr: true,
          force_ocr: false,
          ocr_engine: "rapidocr",
          ocr_lang: ["en"],
        },
        enrichment: {
          do_code_enrichment: false,
          do_formula_enrichment: false,
          do_picture_description: true,
        },
        vlm: {
          provider_account_key: "openrouter-default",
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
        conversion: {
          pdf_backend: "dlparse_v2",
        },
        ocr: {
          do_ocr: true,
          force_ocr: false,
          ocr_engine: "rapidocr",
          ocr_lang: ["en"],
        },
        enrichment: {
          do_code_enrichment: false,
          do_formula_enrichment: false,
          do_picture_description: true,
        },
        vlm: {
          provider_account_key: "openrouter-default",
          vlm_pipeline_model: "gemini-3-flash",
          picture_description_model: "gpt-4o-mini",
          code_formula_model: "gpt-4o-mini",
        },
      },
      signal: undefined,
    });
  });
});
