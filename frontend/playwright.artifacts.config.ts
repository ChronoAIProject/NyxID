import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
// An isolated server and port keep artifact acceptance in this checkout.
export default defineConfig({
  ...base,
  testMatch: ["generated-artifacts.spec.ts"],
  workers: 1,
  use: { ...base.use, baseURL: "http://localhost:49237" },
  webServer: {
    command: "npm run dev -- --mode test --port 49237 --strictPort",
    url: "http://localhost:49237",
    reuseExistingServer: false,
    timeout: 90_000,
  },
});
