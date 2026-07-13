/// <reference types="vitest/config" />

import tailwindcss from "@tailwindcss/vite";
import ui from "@nuxt/ui/vite";
import vue from "@vitejs/plugin-vue";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiTarget = env.VITE_API_TARGET || "http://127.0.0.1:8096";

  return {
    plugins: [
      vue(),
      tailwindcss(),
      ui(),
    ],
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
      setupFiles: ["./src/test-utils/setup.ts"],
    },
  };
});
