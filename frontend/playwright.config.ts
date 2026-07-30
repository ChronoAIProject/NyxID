import { defineConfig, devices } from "@playwright/test";

/**
 * E2E harness for the assistant chat flows (docs/chat-flow-audit.md).
 *
 * Runs the REAL app in Vite dev mode against the scripted MockAssistantTransport
 * (`/assistant?mock` — see src/lib/assistant/transport.ts). No backend and no
 * auth are required: the mock beforeLoad seeds a mock user, and every flow is
 * deterministic (scripted 100 ms event cadence).
 *
 * Dedicated strict port so parallel worktrees racing for :3000 can never serve
 * a different checkout to these tests (see reference_vite_worktree_collision).
 *
 * One command: `npm run test:e2e` (from frontend/).
 */
const PORT = 4611;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  workers: 3,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [["list"]],
  timeout: 45_000,
  use: {
    baseURL: `http://localhost:${String(PORT)}`,
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `npm run dev -- --port ${String(PORT)} --strictPort`,
    url: `http://localhost:${String(PORT)}`,
    reuseExistingServer: !process.env.CI,
    timeout: 90_000,
  },
});
