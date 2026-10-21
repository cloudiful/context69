import type { BrowserContext, Route } from "@playwright/test";

import type { components } from "../../src/generated/openapi";

type AuthMeResponse = components["schemas"]["AuthMeResponse"];
type DoclingSettingsResponse = components["schemas"]["DoclingSettingsResponse"];
type PersonalAccessTokenPageResponse = components["schemas"]["PersonalAccessTokenPageResponse"];
type RuntimeSettingsResponse = components["schemas"]["RuntimeSettingsResponse"];
type SearchSettingsResponse = components["schemas"]["SearchSettingsResponse"];
type TaskPageResponse = components["schemas"]["TaskPageResponse"];

export const TIME_ZONE_BASE_URL = "http://127.0.0.1:5199";
export const FIXED_INSTANT = "2026-03-01T00:00:00.000Z";

export function makeAuthMeResponse(): AuthMeResponse {
  return {
    user: {
      display_name: "Time zone fixture",
      is_admin: false,
      login_name: "time-zone-fixture",
      personal_group_path: "/time-zone",
      user_id: 71,
    },
  };
}

export function makeRuntimeSettings(): RuntimeSettingsResponse {
  return {
    chunking: { max_chars: 1200, overlap_chars: 100 },
    embedding: {
      base_url: "http://127.0.0.1:11434/v1",
      dimensions: 1024,
      has_api_key: false,
      model: "bge-m3",
      timeout_secs: 60,
    },
    file_library: {
      ingest_concurrency: 2,
      max_upload_request_size_mb: 100,
      max_upload_size_mb: 50,
      s3: null,
      storage_root: "/tmp/context69-fixture",
      trusted_proxy_enabled: false,
      url_import_concurrency: 2,
      url_import_min_interval_ms: 1000,
    },
    qdrant: {
      collection_name: "context69",
      recreate_on_dimension_mismatch: false,
      url: "http://127.0.0.1:6333",
    },
    scheduler: {
      interval_secs: 60,
      job_id: "context69-scheduler",
      max_concurrency: 4,
      run_on_start: false,
      valkey_url: null,
    },
  };
}

export function makeDoclingSettings(): DoclingSettingsResponse {
  return {
    configured: false,
    connection: {
      base_url: null,
      max_inflight: 2,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
      timeout_secs: 120,
    },
    source: "unconfigured",
    vlm: { has_api_key: false },
  };
}

export function makeSearchSettings(): SearchSettingsResponse {
  return {
    candidate_limit: 20,
    has_api_key: false,
    keyword_weight: 0.5,
    mode: "hybrid",
    rerank_base_url: "",
    rerank_enabled: false,
    rerank_model: "",
    timeout_secs: 10,
    vector_weight: 0.5,
  };
}

export function makeEmptyTaskPage(): TaskPageResponse {
  return {
    items: [],
    pagination: { page: 1, page_size: 1, total: 0, total_pages: 0 },
  };
}

export function makePersonalAccessTokenPage(): PersonalAccessTokenPageResponse {
  return {
    items: [{
      created_at: FIXED_INSTANT,
      display_prefix: "ctx_pat_fixt",
      expires_at: "2027-03-01T00:00:00.000Z",
      last_used_at: null,
      name: "Time zone fixture",
      revoked_at: null,
      scopes: ["search"],
      token_id: "00000000-0000-4000-8000-000000000071",
      updated_at: FIXED_INSTANT,
    }],
    pagination: { page: 1, page_size: 50, total: 1, total_pages: 1 },
  };
}

export class TimeZoneMockTransport {
  readonly unmatchedApiRequests: string[] = [];
  readonly blockedExternalRequests: string[] = [];

  async install(context: BrowserContext, baseURL: string): Promise<void> {
    const localOrigin = new URL(baseURL).origin;

    await context.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== localOrigin) {
        this.blockedExternalRequests.push(`${route.request().method()} ${url.origin}${url.pathname}`);
        await route.abort("blockedbyclient");
        return;
      }

      if (url.pathname.startsWith("/v1/")) {
        const handled = await this.handleApiRoute(route, url);
        if (handled) {
          return;
        }
        this.unmatchedApiRequests.push(`${route.request().method()} ${url.pathname}`);
        await route.abort("blockedbyclient");
        return;
      }

      await route.continue();
    });
  }

  private async handleApiRoute(route: Route, url: URL): Promise<boolean> {
    const method = route.request().method();

    if (method === "GET" && url.pathname === "/v1/auth/me") {
      await this.fulfill(route, makeAuthMeResponse());
      return true;
    }
    if (method === "GET" && url.pathname === "/v1/settings/runtime") {
      await this.fulfill(route, makeRuntimeSettings());
      return true;
    }
    if (method === "GET" && url.pathname === "/v1/settings/docling") {
      await this.fulfill(route, makeDoclingSettings());
      return true;
    }
    if (method === "GET" && url.pathname === "/v1/settings/search") {
      await this.fulfill(route, makeSearchSettings());
      return true;
    }
    if (method === "GET" && url.pathname === "/v1/tasks") {
      await this.fulfill(route, makeEmptyTaskPage());
      return true;
    }
    if (method === "GET" && url.pathname === "/v1/auth/personal-access-tokens") {
      await this.fulfill(route, makePersonalAccessTokenPage());
      return true;
    }

    return false;
  }

  private async fulfill(route: Route, body: unknown): Promise<void> {
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
  }
}
