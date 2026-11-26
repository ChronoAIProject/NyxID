import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

const port = 4635;
export default defineConfig({
  ...base,
  testMatch: "composer-connection-availability.spec.ts",
  workers: 1,
  use: { ...base.use, baseURL: `http://localhost:${port}` },
  webServer: {
    ...base.webServer,
    command: `npm run dev -- --config vite.e2e.config.ts --mode test --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: false,
  },
});
