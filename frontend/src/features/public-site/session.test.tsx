import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { QueryClient } from "@tanstack/react-query";
import { startPublicSession } from "./session";

const runtime = vi.hoisted(() => ({
  auth: {
    isAuthenticated: false,
    user: null as { id: string } | null,
    checkAuth: vi.fn(async () => {}),
  },
  consent: { asked: true, enabled: false },
  initTelemetry: vi.fn(),
  subscribeCrashPermission: vi.fn(),
}));

vi.mock("react-dom/client", () => ({
  createRoot: () => ({ render: vi.fn() }),
}));
vi.mock("@/components/consent-banner", () => ({ ConsentBanner: () => null }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: { getState: () => runtime.auth, subscribe: vi.fn() },
}));
vi.mock("@/stores/consent-store", () => ({
  useConsentStore: { getState: () => runtime.consent, subscribe: vi.fn() },
}));
vi.mock("@/lib/telemetry", () => ({
  initTelemetry: runtime.initTelemetry,
  capture: vi.fn(),
  identify: vi.fn(),
}));
vi.mock("@/lib/crash-reporting/install", () => ({
  installCrashReporting: vi.fn(),
  crashRootOptions: {},
}));
vi.mock("@/lib/crash-reporting/permission-refresh", () => ({
  subscribeCrashPermission: runtime.subscribeCrashPermission,
}));
vi.mock("@/features/docs/docs-layout-view", () => ({ updatePublicDocsRuntime: vi.fn() }));
vi.mock("./docs-theme", () => ({ PublicDocsTheme: () => null }));

function publicRouter() {
  return { subscribe: vi.fn() } as unknown as Parameters<typeof startPublicSession>[0];
}

beforeEach(() => {
  vi.clearAllMocks();
  runtime.auth.isAuthenticated = false;
  runtime.auth.user = null;
  runtime.consent.asked = true;
  runtime.consent.enabled = false;
  window.history.replaceState({}, "", "/nyxbot");
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  window.history.replaceState({}, "", "/");
});

it("keeps declined public sessions free of configuration and diagnostic requests", async () => {
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);

  await startPublicSession(publicRouter());

  expect(runtime.auth.checkAuth).toHaveBeenCalledOnce();
  expect(fetch).not.toHaveBeenCalled();
  expect(runtime.initTelemetry).not.toHaveBeenCalled();
});

it("passes the public discovery hint to the existing crash permission gate", async () => {
  runtime.consent.enabled = true;
  const config = { crash_diagnostics_offered: true, telemetry_share_analytics: false };
  const fetch = vi.fn(async () => new Response(JSON.stringify(config)));
  vi.stubGlobal("fetch", fetch);

  await startPublicSession(publicRouter());

  expect(fetch).toHaveBeenCalledExactlyOnceWith("/api/v1/public/config");
  expect(runtime.subscribeCrashPermission).toHaveBeenCalledOnce();
  const client = runtime.subscribeCrashPermission.mock.calls[0]![0] as QueryClient;
  expect(client.getQueryData(["public-config"])).toEqual(config);
});

it.each(["/", "/zh"])("retains dev's signed-in Assistant entry from %s", async path => {
  window.history.replaceState({}, "", path);
  runtime.auth.isAuthenticated = true;
  runtime.auth.user = { id: "example-user" };
  const replace = vi.spyOn(window.location, "replace").mockImplementation(() => {});

  await startPublicSession(publicRouter());

  expect(replace).toHaveBeenCalledWith("/assistant");
});
