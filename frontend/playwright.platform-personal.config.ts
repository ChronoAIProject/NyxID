import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// Use the actual HTTP client rather than mode=test's in-app assistant fixtures.
export default defineConfig({
  ...base,
  testMatch: "platform-personal.spec.ts",
  testIgnore: [],
  workers: 1,
  webServer: {
    command: "npm run dev -- --mode development --port 4611 --strictPort",
    url: "http://localhost:4611",
    reuseExistingServer: false,
    timeout: 90000,
  },
});
