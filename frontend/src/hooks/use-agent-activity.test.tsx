import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import type { ProcessState } from "../../../shared/agent-avatars/model";
import { ApiError } from "@/lib/api-error";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { resetNyxAgentLiveForTests } from "@/lib/assistant/nyxagent-live";
import { assistantAgentSchema } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import {
  rosterMoodFor,
  useAgentActivity,
  usePublishAgentActivity,
  useRosterActivity,
} from "./use-agent-activity";
import { nyxAgentLiveRefresh, useNyxAgentLive } from "./use-nyxagent-live";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";

const client = new QueryClient();
const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={client}>{children}</QueryClientProvider>
);
const row = assistantAgentSchema.parse({
  id: "agent",
  name: "Helper",
  kind: "specialist",
  status: "running",
  activity: [
    { conversation_id: "a", state: "working" },
    { conversation_id: "b", state: "waiting" },
  ],
  created_at: "2026-11-25",
  last_active_at: "2026-11-25",
});
const user = {
  id: "owner",
  email: "owner@example.com",
  display_name: "Owner",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-11-25T00:00:00Z",
  capabilities: {
    billing_available: false,
    enabled_features: ["assistant:avatars"],
  },
};
const queryKey = nyxBotQueryKeys.activity(user.id);

afterEach(() => {
  cleanup();
  client.clear();
  vi.restoreAllMocks();
  vi.useRealTimers();
  resetNyxAgentLiveForTests();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

it("keeps optional roster metadata compatible and drops unknown states", () => {
  expect(
    assistantAgentSchema.parse({ ...row, activity: undefined }).activity,
  ).toBeUndefined();
  const parsed = assistantAgentSchema.parse({
    ...row,
    activity: [...row.activity!, { conversation_id: "c", state: "future" }],
  });
  expect(parsed.activity).toEqual(row.activity);
});

it("uses the freshest complete snapshot before applying the open thread", () => {
  const snapshot = { updatedAt: 25, agents: new Map() };
  expect(rosterMoodFor(row, 10, snapshot)).toBe("idle");
  expect(rosterMoodFor(row, 30, snapshot)).toBe("working");
  expect(
    rosterMoodFor(row, 30, snapshot, {
      agentId: row.id,
      conversationId: "a",
      state: "stopping",
    }),
  ).toBe("waiting");
  expect(
    rosterMoodFor({
      ...row,
      activity: undefined,
      status: "idle",
      pending_acknowledgements: 1,
    }),
  ).toBe("approval");
  expect(
    rosterMoodFor({ ...row, activity: [], pending_acknowledgements: 1 }),
  ).toBe("idle");
});

it("turns a 404 into a disabled lookup and continues with roster activity", async () => {
  useAuthStore.getState().setUser(user);
  const request = vi.spyOn(nyxBotApi, "activity").mockRejectedValue(
    new ApiError(404, {
      error: "not_found",
      error_code: 0,
      message: "Not found",
    }),
  );
  const view = renderHook(
    () => ({
      query: useAgentActivity(),
      mood: useRosterActivity(10)(row),
    }),
    { wrapper },
  );
  await waitFor(() => expect(view.result.current.query.data).toBeNull());
  await act(async () => {
    await client.invalidateQueries({ queryKey });
  });
  expect(view.result.current.query.isEnabled).toBe(false);
  expect(view.result.current.mood).toBe("working");
  expect(request).toHaveBeenCalledTimes(1);
});

it("clears exact live overrides on navigation, settlement, unmount and account switch", async () => {
  useAuthStore.getState().setUser(user);
  const request = vi
    .spyOn(nyxBotApi, "activity")
    .mockResolvedValue({ agents: [] });
  client.setQueryData(queryKey, {
    agents: [{ agent_id: row.id, threads: row.activity }],
  });
  const observer = renderHook(() => useRosterActivity(0)(row), { wrapper });
  expect(observer.result.current).toBe("working");
  const publisher = renderHook(
    ({
      conversation,
      mood,
    }: {
      conversation: string | undefined;
      mood: ProcessState;
    }) => usePublishAgentActivity(user.id, conversation, row.id, mood),
    { initialProps: { conversation: "a", mood: "drafting" } },
  );
  expect(observer.result.current).toBe("drafting");
  publisher.rerender({ conversation: "a", mood: "retry" });
  expect(observer.result.current).toBe("retry");
  publisher.rerender({ conversation: "a", mood: "success" });
  expect(observer.result.current).toBe("waiting");
  publisher.rerender({ conversation: "b", mood: "approval" });
  expect(observer.result.current).toBe("approval");
  publisher.rerender({ conversation: undefined, mood: "idle" });
  expect(observer.result.current).toBe("working");
  publisher.rerender({ conversation: "b", mood: "approval" });
  publisher.unmount();
  expect(observer.result.current).toBe("working");
  const oldAccountPublisher = renderHook(() =>
    usePublishAgentActivity(user.id, "a", row.id, "working"),
  );
  act(() => useAuthStore.getState().setUser({ ...user, id: "other" }));
  await waitFor(() => expect(observer.result.current).toBe("idle"));
  expect(client.getQueryData(nyxBotQueryKeys.activity("other"))).toEqual({
    agents: [],
  });
  expect(request).toHaveBeenCalledTimes(1);
  oldAccountPublisher.unmount();
});

it("refreshes activity for same-turn changes without invalidating the rosters", () => {
  const seen = new Map();
  const turn = {
    type: "conversation",
    id: "thread",
    group_id: null,
    turn_id: "turn",
    messages: 2,
    title_changed: false,
  } as const;
  const first = nyxAgentLiveRefresh(user.id, turn, seen);
  expect(first.activity).toBe(true);
  expect(first.now).not.toContainEqual(queryKey);
  expect(first.lists).toEqual([
    nyxBotQueryKeys.threads(user.id),
    nyxBotQueryKeys.agents(user.id),
  ]);
  const next = nyxAgentLiveRefresh(user.id, turn, seen);
  expect(next.activity).toBe(true);
  expect(next.now).not.toContainEqual(queryKey);
  expect(next.lists).toEqual([]);
  expect(
    nyxAgentLiveRefresh(user.id, { type: "group", id: "crew" }, seen).activity,
  ).toBe(true);
  expect(
    nyxAgentLiveRefresh(user.id, { type: "notification", id: "card" }, seen)
      .activity,
  ).toBe(true);
});

function installLiveFixture() {
  let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    expect(endpoint).toBe("/assistant/nyxagent/live");
    return new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          stream = controller;
        },
      }),
      { headers: { "content-type": "text/event-stream" } },
    );
  };
  return {
    isOpen: () => Boolean(stream),
    send(event: object) {
      stream!.enqueue(
        new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
      );
    },
  };
}

it("coalesces streaming writes on a separate one-second cadence and defers an in-flight refresh", async () => {
  useAuthStore.getState().setUser(user);
  const state = { agents: [] };
  let resolveFirst!: (value: typeof state) => void;
  const first = new Promise<typeof state>((resolve) => {
    resolveFirst = resolve;
  });
  const request = vi
    .spyOn(nyxBotApi, "activity")
    .mockReturnValueOnce(first)
    .mockResolvedValue(state);
  const live = installLiveFixture();
  const view = renderHook(
    () => {
      useNyxAgentLive(true);
      return useAgentActivity();
    },
    { wrapper },
  );
  await waitFor(() => expect(live.isOpen()).toBe(true));
  expect(request).toHaveBeenCalledTimes(1);
  const invalidate = vi.spyOn(client, "invalidateQueries");
  vi.useFakeTimers();
  const event = {
    type: "conversation",
    id: `nyxa-${"a".repeat(32)}`,
    group_id: null,
    turn_id: "turn",
    messages: 1,
  };
  await act(async () => {
    for (let index = 0; index < 3; index += 1) {
      live.send(event);
    }
    await vi.advanceTimersByTimeAsync(150);
  });
  expect(invalidate).toHaveBeenCalledWith(
    { queryKey: nyxBotQueryKeys.history(user.id, event.id) },
    expect.anything(),
  );
  expect(invalidate).not.toHaveBeenCalledWith(
    { queryKey, exact: true },
    expect.anything(),
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(850);
  });
  expect(request).toHaveBeenCalledTimes(1);
  await act(async () => {
    resolveFirst(state);
    await vi.advanceTimersByTimeAsync(999);
  });
  expect(request).toHaveBeenCalledTimes(1);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(request).toHaveBeenCalledTimes(2);
  expect(view.result.current.isFetching).toBe(false);
});

it("retries an unavailable activity endpoint once on a live resync", async () => {
  useAuthStore.getState().setUser(user);
  const missing = new ApiError(404, {
    error: "not_found",
    error_code: 0,
    message: "Not found",
  });
  const request = vi.spyOn(nyxBotApi, "activity").mockRejectedValue(missing);
  const live = installLiveFixture();
  const view = renderHook(
    () => {
      useNyxAgentLive(true);
      return useAgentActivity();
    },
    { wrapper },
  );
  await waitFor(() =>
    expect(live.isOpen() && view.result.current.data === null).toBe(true),
  );
  vi.useFakeTimers();
  await act(async () => {
    live.send({ type: "ready" });
    live.send({ type: "resync" });
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(request).toHaveBeenCalledTimes(2);
  expect(view.result.current.data).toBeNull();
  await act(async () => {
    live.send({ type: "notification", id: "card" });
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(request).toHaveBeenCalledTimes(2);
});
