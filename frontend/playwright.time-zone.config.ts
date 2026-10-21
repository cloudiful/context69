import { join } from "node:path";
import { defineConfig } from "@playwright/test";

import { TIME_ZONE_BASE_URL } from "./e2e/time-zone/fixtures";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "time-zone/preferences.spec.ts",
  outputDir: join(".cache/ai/1007", "playwright-results"),
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 30_000 },
  reporter: "list",
  use: {
    baseURL: TIME_ZONE_BASE_URL,
    browserName: "chromium",
    viewport: { width: 1440, height: 1000 },
    timezoneId: "UTC",
    serviceWorkers: "block",
    trace: "on",
    screenshot: "off",
    video: "off",
  },
  webServer: {
    command: "bun x vite preview --host 127.0.0.1 --port 5199 --strictPort",
    url: TIME_ZONE_BASE_URL,
    reuseExistingServer: false,
    timeout: 120_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
