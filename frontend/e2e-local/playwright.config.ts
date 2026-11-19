import { defineConfig } from "@playwright/test";

// Local backend only; intentionally separate from the HTTP-fixture E2E suite.
// Start Vite with BACKEND_URL and FRONTEND_URL matching that backend, then:
// NYXID_LOCAL_AUTH_ORIGIN=http://127.0.0.1:3122 pnpm exec playwright test --config e2e-local/playwright.config.ts
export default defineConfig({
  testDir: ".",
  workers: 1,
  timeout: 45_000,
  reporter: "list",
  use: { headless: true, viewport: { width: 1200, height: 1000 } },
});
