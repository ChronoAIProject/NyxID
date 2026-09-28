import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  GROUP_IDLE_POLL_MS,
  GROUP_WORKING_POLL_MS,
  groupPollInterval,
  useNyxBotGroupMessages,
  useNyxBotGroups,
} from "./use-nyxbot-groups";
import { NyxAgentHttpFixtures } from "@/lib/assistant/nyxagent-http-fixtures";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import type { AssistantGroupMessages } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";

let client: QueryClient;

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  sessionStorage.clear();
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-09-29T00:00:00Z",
  });
  globalThis.__nyxidAssistantHttpMock = new NyxAgentHttpFixtures().handler;
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});

afterEach(() => {
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
  sessionStorage.clear();
});

describe("groupPollInterval", () => {
  const page = (working: string[]) =>
    ({
      group: { working_agent_ids: working },
      messages: [],
      before_seq: null,
    }) as unknown as AssistantGroupMessages;

  it("polls quickly while an agent works or just after posting, else every 15 s", () => {
    expect(groupPollInterval(page(["a"]), 0, 1000)).toBe(GROUP_WORKING_POLL_MS);
    expect(groupPollInterval(page([]), 2000, 1000)).toBe(GROUP_WORKING_POLL_MS);
    expect(groupPollInterval(page([]), 500, 1000)).toBe(GROUP_IDLE_POLL_MS);
    expect(groupPollInterval(undefined, 0, 1000)).toBe(GROUP_IDLE_POLL_MS);
    expect(GROUP_WORKING_POLL_MS).toBe(1500);
    expect(GROUP_IDLE_POLL_MS).toBe(15_000);
  });
});

async function newGroup() {
  const { agents } = await nyxBotApi.agents(false);
  return nyxBotApi.createGroup({
    name: "Crew",
    member_agent_ids: agents.map((agent) => agent.id),
  });
}

describe("useNyxBotGroupMessages", () => {
  it("shows a posted message and its addressed agents as working straight away", async () => {
    const group = await newGroup();
    const researcher = group.members.find((member) => member.name === "researcher")!;
    const { result, unmount } = renderHook(
      () => ({ transcript: useNyxBotGroupMessages(group.id), list: useNyxBotGroups() }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.transcript.data?.messages).toHaveLength(1));
    await waitFor(() => expect(result.current.list.data).toHaveLength(1));
    await act(async () => {
      await result.current.transcript.post.mutateAsync("@researcher look");
    });
    // Query results are delivered on the next tick.
    await waitFor(() =>
      expect(result.current.transcript.data?.messages.at(-1)).toMatchObject({
        role: "user",
        text: "@researcher look",
      }),
    );
    expect(result.current.transcript.data?.group.working_agent_ids).toEqual([researcher.id]);
    // The list row follows the transcript's view of the group.
    await waitFor(() =>
      expect(result.current.list.data?.[0]?.working_agent_ids).toEqual([researcher.id]),
    );
    unmount();
  });

  it("merges older pages into the cached transcript", async () => {
    const group = await newGroup();
    const everyone = group.members.map((member) => member.id);
    // Each membership change posts a notice: 1 + 55 messages in all.
    for (let index = 0; index < 55; index += 1) {
      await nyxBotApi.updateGroup(group.id, {
        member_agent_ids: index % 2 === 0 ? [group.lead_agent_id] : everyone,
      });
    }
    const { result, unmount } = renderHook(() => useNyxBotGroupMessages(group.id), { wrapper });
    await waitFor(() => expect(result.current.data?.messages).toHaveLength(50));
    expect(result.current.data?.before_seq).toBe(7);
    await act(() => result.current.loadOlder());
    await waitFor(() =>
      expect(result.current.data?.messages.map((message) => message.seq)).toEqual(
        Array.from({ length: 56 }, (_, index) => index + 1),
      ),
    );
    expect(result.current.data?.before_seq).toBeNull();
    // A later poll of the newest page keeps the older messages.
    await act(() => result.current.refetch());
    await waitFor(() => expect(result.current.isFetching).toBe(false));
    expect(result.current.data?.messages).toHaveLength(56);
    expect(result.current.data?.before_seq).toBeNull();
    unmount();
  });
});
