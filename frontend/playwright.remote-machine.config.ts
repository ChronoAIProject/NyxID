import { defineConfig, devices } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  testMatch: "remote-machines.spec.ts",
  workers: 1,
  retries: 0,
  timeout: 30000,
  reporter: [["list"]],
  outputDir: process.env.P4_E2E_OUTPUT_DIR ?? "./test-results/remote-machines",
  use: {
    baseURL: "http://localhost:4627",
    ...devices["Desktop Chrome"],
    channel: process.env.P4_E2E_CHANNEL ?? "chrome",
    trace: "off",
    screenshot: "off",
  },
  webServer: {
    command: "npm run dev -- --mode test --port 4627 --strictPort",
    url: "http://localhost:4627",
    reuseExistingServer: false,
    timeout: 60000,
  },
});
