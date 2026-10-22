import { join } from "node:path";
import { defineConfig } from "@playwright/test";
import { QUEUE_PERF_BASE_URL, queuePerfSettings } from "./e2e/queue-perf/metrics";

const settings = queuePerfSettings();

export default defineConfig({
  testDir: "./e2e",
  testMatch: "processing-queue-perf.spec.ts",
  outputDir: join(settings.outputDir, "playwright-results"),
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  workers: 1,
  timeout: 180_000,
  expect: { timeout: 30_000 },
  reporter: "list",
  use: {
    baseURL: QUEUE_PERF_BASE_URL,
    browserName: "chromium",
    viewport: { width: 1440, height: 1000 },
    serviceWorkers: "block",
    trace: "on",
    screenshot: "off",
    video: "off",
  },
  webServer: {
    command: "bun x vite preview --host 127.0.0.1 --port 5188 --strictPort",
    url: QUEUE_PERF_BASE_URL,
    reuseExistingServer: false,
    timeout: 120_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
