/// <reference types="vitest/config" />

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";
import { defineConfig, loadEnv } from "vite";
import { configDefaults } from "vitest/config";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiTarget = env.VITE_API_TARGET || "http://127.0.0.1:8096";

  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
      },
    },
    server: {
      host: "0.0.0.0",
      port: 5173,
      proxy: {
        "/v1": {
          target: apiTarget,
          changeOrigin: true,
        },
        "/healthz": {
          target: apiTarget,
          changeOrigin: true,
        },
        "/readyz": {
          target: apiTarget,
          changeOrigin: true,
        },
        "/livez": {
          target: apiTarget,
          changeOrigin: true,
        },
        "/openapi.json": {
          target: apiTarget,
          changeOrigin: true,
        },
      },
    },
    test: {
      environment: "jsdom",
      globals: true,
      css: true,
      setupFiles: ["./src/test-utils/setup-react.ts"],
      // Every remaining source test is discovered; only the browser suite and
      // normal build/dependency output stay out of the unit run.
      exclude: [...configDefaults.exclude, "e2e/**"],
    },
  };
});
