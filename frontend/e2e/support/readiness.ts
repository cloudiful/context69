import {
  E2E_API_URL,
  E2E_BASE_URL,
  E2E_READINESS_TIMEOUT_MS,
} from "./env";

type ProbeOutcome =
  | { ok: true }
  | { ok: false; detail: string };

const PROBE_INTERVAL_MS = 1_000;
const PROBE_REQUEST_TIMEOUT_MS = 5_000;

function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.name === "AbortError" ? "request timed out" : error.message;
  }

  return String(error);
}

async function fetchWithTimeout(url: string): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), PROBE_REQUEST_TIMEOUT_MS);

  try {
    return await fetch(url, { signal: controller.signal, redirect: "follow" });
  } finally {
    clearTimeout(timer);
  }
}

async function probeFrontend(): Promise<ProbeOutcome> {
  const url = `${E2E_BASE_URL}/`;

  try {
    const response = await fetchWithTimeout(url);
    if (!response.ok) {
      return { ok: false, detail: `unexpected HTTP ${response.status}` };
    }

    const contentType = response.headers.get("content-type") ?? "";
    if (!contentType.includes("text/html")) {
      return { ok: false, detail: `expected HTML, received '${contentType || "unknown"}'` };
    }

    return { ok: true };
  } catch (error) {
    return { ok: false, detail: describeError(error) };
  }
}

/**
 * `/healthz` answers 200 when healthy and 503 when only optional dependencies
 * (Qdrant/embedding/Docling) are down. The smoke suite must not require those
 * services, so readiness accepts a degraded API as long as the database is up.
 */
async function probeBackend(): Promise<ProbeOutcome> {
  const url = `${E2E_API_URL}/healthz`;

  try {
    const response = await fetchWithTimeout(url);
    const body = (await response.json().catch(() => null)) as {
      status?: unknown;
      db_ok?: unknown;
    } | null;

    if (!body || typeof body !== "object") {
      return { ok: false, detail: `unexpected HTTP ${response.status} without a JSON health body` };
    }

    if (body.status === "ok" || body.db_ok === true) {
      return { ok: true };
    }

    if (body.db_ok === false) {
      return { ok: false, detail: `API reached but the database is unavailable (db_ok=false)` };
    }

    return { ok: false, detail: `API reported status '${String(body.status)}'` };
  } catch (error) {
    return { ok: false, detail: describeError(error) };
  }
}

async function delay(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Polls the frontend and backend until both answer, then returns. Any other
 * outcome throws one actionable error naming each endpoint and how to start
 * the documented disposable stack.
 */
export async function waitForReadiness(timeoutMs = E2E_READINESS_TIMEOUT_MS): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let frontend: ProbeOutcome = { ok: false, detail: "not checked yet" };
  let backend: ProbeOutcome = { ok: false, detail: "not checked yet" };

  while (true) {
    [frontend, backend] = await Promise.all([probeFrontend(), probeBackend()]);

    if (frontend.ok && backend.ok) {
      return;
    }

    if (Date.now() >= deadline) {
      break;
    }

    await delay(PROBE_INTERVAL_MS);
  }

  const lines = [
    `E2E readiness check failed after ${timeoutMs}ms.`,
    `  - frontend ${E2E_BASE_URL}/ -> ${frontend.ok ? "ok" : frontend.detail}`,
    `  - backend  ${E2E_API_URL}/healthz -> ${backend.ok ? "ok" : backend.detail}`,
    "",
    "Start the disposable local stack first:",
    "  ./scripts/dev.sh full",
    "",
    "Override endpoints with E2E_BASE_URL and E2E_API_URL.",
    "Override credentials with E2E_USERNAME and E2E_PASSWORD.",
  ];

  throw new Error(lines.join("\n"));
}
