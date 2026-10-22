import { defineConfig, devices } from "@playwright/test";

/** Owned preview port so the suite never touches the dev stack on 5173. */
export const E2E_PORT = 5199;
export const E2E_BASE_URL = `http://127.0.0.1:${E2E_PORT}`;

/**
 * The default browser suite.
 *
 * It serves a built production preview with local, generated-contract API
 * mocks and a fixed UTC browser, so it needs no backend, database or
 * production access. Vitest stays the component/unit runner and Rust stays the
 * API/integration runner.
 */
export default defineConfig({
  testDir: "./e2e/react",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: E2E_BASE_URL,
    timezoneId: "UTC",
    locale: "en-US",
    serviceWorkers: "block",
    trace: "retain-on-failure",
  },
  webServer: {
    command: `bun x vite preview --host 127.0.0.1 --port ${E2E_PORT} --strictPort`,
    url: E2E_BASE_URL,
    reuseExistingServer: false,
    timeout: 120_000,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
