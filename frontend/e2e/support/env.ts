/**
 * Environment contract for the Playwright E2E harness.
 *
 * Every value is environment-driven so the suite targets whatever disposable
 * local/test stack the operator started. The defaults match the documented dev
 * stack (`./scripts/dev.sh full`) and its disposable bootstrap account.
 */
function readString(name: string, fallback: string): string {
  const value = process.env[name];
  return value && value.trim().length > 0 ? value.trim() : fallback;
}

function readPositiveInt(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }

  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export const E2E_BASE_URL = readString("E2E_BASE_URL", "http://127.0.0.1:5173").replace(/\/+$/, "");

export const E2E_API_URL = readString("E2E_API_URL", "http://127.0.0.1:8096").replace(/\/+$/, "");

export const E2E_USERNAME = readString("E2E_USERNAME", "admin");

/** Documented disposable bootstrap password; override for any real environment. */
export const E2E_PASSWORD = readString("E2E_PASSWORD", "change-me-now");

/** Generated at test time and never committed (see frontend/.gitignore). */
export const E2E_STORAGE_STATE = readString("E2E_STORAGE_STATE", "e2e/.auth/user.json");

export const E2E_READINESS_TIMEOUT_MS = readPositiveInt("E2E_READINESS_TIMEOUT_MS", 60_000);
