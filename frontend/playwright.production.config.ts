import { defineConfig, devices } from "@playwright/test";

const baseURL = process.env.NYXID_PRODUCTION_TEST_URL;
if (!baseURL) throw new Error("Start the production nginx harness and set NYXID_PRODUCTION_TEST_URL");

export default defineConfig({
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [["list"], ["json", { outputFile: process.env.NYXID_PRODUCTION_RESULTS ?? "test-results/production-policy.json" }]],
  timeout: 45_000,
  use: {
    ...devices["Desktop Chrome"], baseURL,
    permissions: ["microphone"],
    launchOptions: {
      executablePath: process.env.NYXID_CHROME_EXECUTABLE,
      args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
    },
    trace: "retain-on-failure",
  },
  // These reuse transport fixtures with the actual built SPA. There is no
  // Vite server, mock query flag, or dev-only module in this configuration.
  projects: [
    { name: "production-policy", testDir: "./e2e-production" },
    { name: "production-oauth-meta", testDir: "./e2e", testMatch: /channel-(x|whatsapp)-managed\.spec\.ts/,
      grep: /X OAuth connect and reconnect at 1440px|managed Meta (?:Cloud API signup|signup) at 1440px/ },
    { name: "production-telegram-privacy", testDir: "./e2e", testMatch: /channel-telegram-new\.spec\.ts/,
      grep: /Telegram secrets stay out of live telemetry and storage/ },
  ],
});
