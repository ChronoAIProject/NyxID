import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useCollaborationAvailable, useCollaborationRead } from "./use-collaboration-groups";
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


it("marks the opened transcript read, monotonically, but skips legacy and hidden groups", async () => {
  flags.enabled = true;
  const positions: number[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    calls.push(endpoint);
    positions.push((JSON.parse(String(init?.body)) as { last_seen_seq: number }).last_seen_seq);
    return new Response(null, { status: 204 });
  };
  const { rerender } = renderHook(({ seq, enabled }) => useCollaborationRead("nyxg-group", seq, enabled), { wrapper, initialProps: { seq: 5, enabled: false } });
  expect(calls).toEqual([]);
  rerender({ seq: 5, enabled: true });
  await waitFor(() => expect(positions).toEqual([5]));
  rerender({ seq: 3, enabled: true });
  expect(positions).toEqual([5]);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  rerender({ seq: 8, enabled: true });
  expect(positions).toEqual([5]);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  act(() => document.dispatchEvent(new Event("visibilitychange")));
  await waitFor(() => expect(positions).toEqual([5, 8]));
  expect(calls).toEqual(["/assistant/nyxagent/groups/nyxg-group/read", "/assistant/nyxagent/groups/nyxg-group/read"]);
  vi.restoreAllMocks();
});
