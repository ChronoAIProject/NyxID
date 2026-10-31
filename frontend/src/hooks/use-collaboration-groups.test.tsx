import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useCollaborationAvailable } from "./use-collaboration-groups";
import { useAuthStore } from "@/stores/auth-store";
const flags = vi.hoisted(() => ({ enabled: false }));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => flags.enabled,
}));
let client: QueryClient;
let calls: string[];
beforeEach(() => {
  flags.enabled = false;
  calls = [];
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore
    .getState()
    .setUser({
      id: "owner",
      email: "owner@example.com",
      display_name: "Owner",
      avatar_url: null,
      email_verified: true,
      mfa_enabled: false,
      is_admin: false,
      is_active: true,
      created_at: "2026-10-31T00:00:00Z",
    });
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    calls.push(endpoint);
    return new Response(JSON.stringify({ available: false }));
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  useAuthStore.getState().setUser(null);
  globalThis.__nyxidAssistantHttpMock = undefined;
});
const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={client}>{children}</QueryClientProvider>
);
it("makes no capability request while the shared-agents flag is off", () => {
  const { result } = renderHook(useCollaborationAvailable, { wrapper });
  expect(result.current.fetchStatus).toBe("idle");
  expect(calls).toEqual([]);
});
it("requires backend readiness even when the per-person flag is enabled", async () => {
  flags.enabled = true;
  const { result } = renderHook(useCollaborationAvailable, { wrapper });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data).toBe(false);
  expect(calls).toEqual(["/assistant/nyxagent/collaboration"]);
});
it("enables creation when the signed-in person's flag and readiness both allow it", async () => {
  flags.enabled = true;
  globalThis.__nyxidAssistantHttpMock = () =>
    new Response(JSON.stringify({ available: true }));
  const { result } = renderHook(useCollaborationAvailable, { wrapper });
  await waitFor(() => expect(result.current.data).toBe(true));
});

it("hides cached availability immediately when the flag is revoked", async () => {
  flags.enabled = true;
  globalThis.__nyxidAssistantHttpMock = () =>
    new Response(JSON.stringify({ available: true }));
  const { result, rerender } = renderHook(useCollaborationAvailable, {
    wrapper,
  });
  await waitFor(() => expect(result.current.data).toBe(true));
  flags.enabled = false;
  rerender();
  expect(result.current.data).toBe(false);
});
