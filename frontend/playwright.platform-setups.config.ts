import { defineConfig, devices } from "@playwright/test";

// Real main.tsx (including StrictMode), router and dashboard; only HTTP is faked.
// Never reuse another checkout's server or forward fixture misses to a backend.
export default defineConfig({
  testDir: "./e2e",
  testMatch: "platform-setups.spec.ts",
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  timeout: 30_000,
  expect: { timeout: 8_000 },
  reporter: [["list"]],
  outputDir: "./test-results/platform-setups",
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://localhost:4634",
    locale: "en-US",
    serviceWorkers: "block",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: {
    command:
      "BACKEND_URL=http://127.0.0.1:9 npm run dev -- --mode platform-setups-acceptance --host localhost --port 4634 --strictPort",
    url: "http://localhost:4634",
    reuseExistingServer: false,
    timeout: 90_000,
  },
});
