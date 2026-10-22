import { defineConfig, devices } from "@playwright/test";

// This suite runs the real app against browser-intercepted fixture APIs only.
// A dead proxy target also protects against accidental ingress outside routing.
export default defineConfig({
  testDir: "./e2e",
  testMatch: "machine-detail.spec.ts",
  workers: 1,
  retries: 0,
  timeout: 30000,
  expect: { timeout: 6000 },
  reporter: [["list"]],
  outputDir: process.env.P4_E2E_OUTPUT_DIR ?? "./test-results/machine-detail",
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://localhost:4628",
    channel: process.env.P4_E2E_CHANNEL ?? "chrome",
    serviceWorkers: "block",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: {
    command:
      "BACKEND_URL=http://127.0.0.1:9 npm run dev -- --mode machine-acceptance --host localhost --port 4628 --strictPort",
    url: "http://localhost:4628",
    reuseExistingServer: false,
    timeout: 60000,
  },
});
