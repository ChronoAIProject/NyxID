import { defineConfig, devices } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e-public",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 45000,
  reporter: [
    ["list"],
    ["json", { outputFile: "dist/public-browser-results.json" }],
  ],
  use: {
    baseURL: process.env.PUBLIC_TEST_ORIGIN || "http://127.0.0.1:4188",
    ...devices["Desktop Chrome"],
    contextOptions: { reducedMotion: "reduce" },
    trace: "retain-on-failure",
  },
});
