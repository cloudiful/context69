import { defineConfig, devices } from "@playwright/test";

import { E2E_BASE_URL, E2E_STORAGE_STATE } from "./e2e/support/env";

/**
 * Standalone Playwright Test harness for full-browser smoke coverage.
 *
 * Targets an already-running disposable local stack (`./scripts/dev.sh full`);
 * the global setup verifies readiness with actionable output. Vitest stays the
 * component/composable runner and Rust stays the API/integration runner.
 */
export default defineConfig({
  testDir: "./e2e",
  globalSetup: "./e2e/global-setup.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: E2E_BASE_URL,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "setup",
      testMatch: /auth\.setup\.ts/,
    },
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        storageState: E2E_STORAGE_STATE,
      },
      dependencies: ["setup"],
    },
  ],
});
