import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig({
  ...base,
  testMatch: "admin-feature-flags.spec.ts",
  workers: 1,
  use: { ...base.use, baseURL: "http://127.0.0.1:4861" },
  webServer: {
    command:
      "npm run dev -- --host 127.0.0.1 --mode test --port 4861 --strictPort",
    url: "http://127.0.0.1:4861",
    reuseExistingServer: false,
    timeout: 90_000,
  },
});
