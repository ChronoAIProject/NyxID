import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys, useConnectNyxBotChannel, useRetryNyxBotChannelGateway, useCreateNyxBotAgent, useNyxBotAgents } from "./use-nyxbot-agents";
import { useNyxAgentLive } from "./use-nyxagent-live";
import { resetNyxAgentLiveForTests } from "@/lib/assistant/nyxagent-live";
import { useAuthStore } from "@/stores/auth-store";

const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
afterEach(() => {
  client.clear();
  vi.restoreAllMocks();
  resetNyxAgentLiveForTests();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null });
});

function rosterFixture() {
  useAuthStore.getState().setUser({
    id: "owner", email: "owner@example.com", display_name: "Owner", avatar_url: null,
    email_verified: true, mfa_enabled: false, is_admin: false, is_active: true,
    created_at: "2026-11-06T00:00:00Z",
  });
  const row = {
    id: "new-specialist", kind: "specialist", name: "restaurant", description: "Dining",
    specialty: null, created_by: "user", status: "idle", services: [], account_read: false,
    pending_requests: [], last_reply: null, home_conversation_id: `nyxa-${"a".repeat(32)}`,
    memory_count: 0, created_at: "2026-11-06T00:00:00Z", last_active_at: "2026-11-06T00:00:00Z",
    destroyed_at: null,
  };
  const state = { agents: [] as (typeof row)[], reads: 0, destroyedTruncated: false };
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint === "/assistant/nyxagent/agents" && init.method === "POST") {
      state.agents = [row];
      return new Response(JSON.stringify({ id: row.id, name: row.name, home_conversation_id: row.home_conversation_id }));
    }
    if (endpoint === "/assistant/nyxagent/agents?include_destroyed=true") {
      state.reads++;
      return new Response(JSON.stringify({ agents: state.agents, destroyed_truncated: state.destroyedTruncated, limits: {
        skip_destructive_confirmation: false, max_live_subagents: 8,
        max_concurrent_subagent_turns: 3, max_live_subagents_limit: 32,
        max_concurrent_subagent_turns_limit: 8,
      } }));
    }
    throw new Error(`Unexpected request: ${endpoint}`);
  };
  return { state, row };
}

it("discovers an empty-home specialist after creation and a fresh query cache", async () => {
  const { row, state } = rosterFixture();
  const first = renderHook(() => ({ agents: useNyxBotAgents(), create: useCreateNyxBotAgent() }), { wrapper });
  await waitFor(() => expect(first.result.current.agents.isSuccess).toBe(true));
  await act(async () => {
    await first.result.current.create.mutateAsync({ name: row.name, description: row.description, services: [], account_read: false });
  });
  await waitFor(() => expect(first.result.current.agents.data?.agents[0]?.id).toBe(row.id));
  expect(first.result.current.agents.data?.agents[0]?.last_reply).toBeNull();
  first.unmount();
  client.clear();
  const reads = state.reads;
  const reloaded = renderHook(useNyxBotAgents, { wrapper });
  await waitFor(() => expect(reloaded.result.current.data?.agents[0]?.id).toBe(row.id));
  expect(state.reads).toBeGreaterThan(reads);
  reloaded.unmount();
});

it("keeps archive truncation metadata from the roster response", async () => {
  const { state } = rosterFixture();
  state.destroyedTruncated = true;
  const view = renderHook(useNyxBotAgents, { wrapper });
  await waitFor(() => expect(view.result.current.data?.destroyed_truncated).toBe(true));
  view.unmount();
});

it("discovers a tool-created specialist through its empty-home live event", async () => {
  const { state, row } = rosterFixture();
  const http = globalThis.__nyxidAssistantHttpMock!;
  let stream!: ReadableStreamDefaultController<Uint8Array>;
  const encoder = new TextEncoder();
  globalThis.__nyxidAssistantHttpMock = (request) => request.endpoint === "/assistant/nyxagent/live"
    ? new Response(new ReadableStream<Uint8Array>({ start(controller) { stream = controller; } }), { headers: { "content-type": "text/event-stream" } })
    : http(request);
  const view = renderHook(() => { useNyxAgentLive(true); return useNyxBotAgents(); }, { wrapper });
  await waitFor(() => expect(view.result.current.isSuccess).toBe(true));
  state.agents = [row];
  act(() => {
    stream.enqueue(encoder.encode(`data: ${JSON.stringify({ type: "conversation", id: row.home_conversation_id, group_id: null, turn_id: null, messages: 0 })}\n\n`));
  });
  await waitFor(() => expect(view.result.current.data?.agents[0]?.id).toBe(row.id), { timeout: 4000 });
  view.unmount();
});

it("invalidates channels and agents after a rejected connect", async () => {
  const key = nyxBotQueryKeys.channels(undefined);
  client.setQueryData(key, []);
  vi.spyOn(nyxBotApi, "connectChannel").mockRejectedValue(new Error("Gateway refused"));
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const { result, unmount } = renderHook(useConnectNyxBotChannel, { wrapper });
  await act(async () => {
    await expect(result.current.mutateAsync({ botId: "bot", agentId: "agent" })).rejects.toThrow("Gateway refused");
  });
  expect(invalidate).toHaveBeenCalledWith({ queryKey: key });
  expect(invalidate).toHaveBeenCalledWith({ queryKey: nyxBotQueryKeys.agents(undefined) });
  expect(client.getQueryState(key)?.isInvalidated).toBe(true);
  unmount();
});

it("2183 refreshes actual transport and activation after a retry", async () => {
  vi.spyOn(nyxBotApi, "retryChannelGateway").mockResolvedValue({ code: "gateway_rejected_request" });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const { result, unmount } = renderHook(useRetryNyxBotChannelGateway, { wrapper });
  await act(async () => { await result.current.mutateAsync({ channelAgentId: "channel" }); });
  expect(nyxBotApi.retryChannelGateway).toHaveBeenCalledWith("channel");
  expect(invalidate).toHaveBeenCalledWith({ queryKey: nyxBotQueryKeys.channels(undefined) });
  unmount();
});
