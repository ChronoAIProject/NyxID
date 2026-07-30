/**
 * Regression probes from the chat-flow audit, docs/chat-flow-audit.md.
 *
 * They drive the real hooks against the real mock transport, plus targeted
 * spies for live-transport event vocabulary and failure paths.
 */
import { act, render, renderHook, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import {
  assistantTransport,
  resetAssistantTransport,
} from "@/lib/assistant/transport";
import { ChatThread } from "@/components/assistant/chat-thread";
import type { AssistantMessage, TurnEvent } from "@/types/assistant";
import {
  useAssistantTurn,
  useDecideApproval,
  useSendMessage,
  useTurnEpisode,
} from "./use-assistant";

const TEST_NOW = Date.parse("2026-07-16T04:00:00.000Z");

function createHarness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { queryClient, Wrapper };
}

beforeEach(() => {
  vi.useFakeTimers();
  resetAssistantTransport(() => TEST_NOW);
});

afterEach(() => {
  delete (globalThis as { __assistantMockFaults?: unknown })
    .__assistantMockFaults;
  vi.restoreAllMocks();
  resetAssistantTransport(() => TEST_NOW);
  vi.useRealTimers();
});

describe("episode-slot ownership (NYX-2 / hypothesis P1-a)", () => {
  it("a rejected concurrent send preserves the live stream's episode", async () => {
    // A stream that produced no events yet mirrors the real transport before
    // its response headers arrive.
    (
      globalThis as {
        __assistantMockFaults?: { sendSilent?: boolean };
      }
    ).__assistantMockFaults = { sendSilent: true };

    const { Wrapper } = createHarness();
    const { result } = renderHook(
      () => ({
        send: useSendMessage("conversation-github"),
        episode: useTurnEpisode("conversation-github"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.send.mutateAsync("First message.");
      await vi.advanceTimersByTimeAsync(50);
    });
    // The live turn's episode is open — this is what keeps the thinking
    // indicator honest while the stream starts up.
    expect(result.current.episode.data?.open).toBe(true);

    await act(async () => {
      await expect(
        result.current.send.mutateAsync("Second, while the first runs."),
      ).rejects.toBeInstanceOf(AssistantTurnActiveError);
      await vi.advanceTimersByTimeAsync(5_000);
    });

    expect(result.current.episode.data).toEqual({
      open: true,
      printed: false,
      projecting: false,
    });
  });
});

describe("stream deadlines (NYX-1 / NYX-7)", () => {
  it("fails an episode that emits no event within the start deadline", async () => {
    (
      globalThis as {
        __assistantMockFaults?: { sendSilent?: boolean };
      }
    ).__assistantMockFaults = { sendSilent: true };

    const { Wrapper } = createHarness();
    const { result } = renderHook(
      () => ({
        send: useSendMessage("conversation-github"),
        episode: useTurnEpisode("conversation-github"),
        turn: useAssistantTurn("conversation-github"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.send.mutateAsync("Wait for the deadline.");
      await vi.advanceTimersByTimeAsync(8_001);
    });

    expect(result.current.turn.data).toMatchObject({
      status: "failed",
      error: { code: "stream_start_timeout" },
    });
    expect(result.current.episode.data).toEqual({
      open: false,
      printed: false,
      projecting: false,
    });
  });

  it("stops reporting projection work when transcript reads never settle", async () => {
    vi.spyOn(assistantTransport, "getHistory").mockImplementation(
      () => new Promise(() => undefined),
    );

    const { Wrapper } = createHarness();
    const { result } = renderHook(
      () => ({
        send: useSendMessage("conversation-github"),
        episode: useTurnEpisode("conversation-github"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      const pending = result.current.send.mutateAsync("Project this reply.");
      await vi.advanceTimersByTimeAsync(5_000);
      await pending;
      await vi.advanceTimersByTimeAsync(2_000);
    });

    expect(result.current.episode.data).toEqual({
      open: false,
      printed: true,
      projecting: false,
    });
  });
});

describe("approval episode cleanup (NYX-5 / hypothesis P2-f)", () => {
  it("a settled approval without a continuation disowns its episode", async () => {
    const { Wrapper } = createHarness();
    const { result } = renderHook(
      () => ({
        decide: useDecideApproval("conversation-stripe"),
        episode: useTurnEpisode("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.decide.mutateAsync({
        blockId: "approval-stripe-lark",
        approved: true,
      });
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.episode.data).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(result.current.episode.data).toBeNull();
  });

  it("a rejected approval restores the prior episode instead of leaking its pump", async () => {
    vi.spyOn(assistantTransport, "decideApproval").mockRejectedValue(
      new Error("Approval transport failed."),
    );

    const { Wrapper } = createHarness();
    const { result } = renderHook(
      () => ({
        decide: useDecideApproval("conversation-stripe"),
        episode: useTurnEpisode("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await expect(
        result.current.decide.mutateAsync({
          blockId: "approval-stripe-lark",
          approved: true,
        }),
      ).rejects.toThrow("Approval transport failed.");
    });

    expect(result.current.episode.data).toBeNull();
  });
});

describe("approval decision events count as printed (NYX-4 / hypothesis P1-c)", () => {
  it("a JSON-ack approval closes the episode as printed", async () => {
    // Script exactly what AevatarAssistantTransport emits on the JSON-ack
    // approve path (aevatar-transport.ts:1285-1349): the decision patch,
    // the parked-ledger patch, then finishTurn's turn.completed. No
    // block.started/delta/completed ever fires.
    vi.spyOn(assistantTransport, "decideApproval").mockImplementation(
      (
        _conversationId: string,
        blockId: string,
        approved: boolean,
        onEvent?: (event: TurnEvent) => void,
      ) => {
        onEvent?.({
          cursor: 1,
          event: "block.updated",
          block_id: blockId,
          patch: {
            decision: approved ? "approved" : "denied",
            decision_channel: "web",
          },
        });
        onEvent?.({
          cursor: 2,
          event: "turn.completed",
          turn_id: "turn-continuation",
          status: "completed",
          error: null,
        });
        return Promise.resolve(null);
      },
    );

    const { Wrapper } = createHarness();
    const { result } = renderHook(
      () => ({
        decide: useDecideApproval("conversation-stripe"),
        episode: useTurnEpisode("conversation-stripe"),
        turn: useAssistantTurn("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.decide.mutateAsync({
        blockId: "approval-stripe-lark",
        approved: true,
      });
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.turn.data?.status).toBe("completed");
    expect(result.current.episode.data).toMatchObject({
      open: false,
      printed: true,
    });
  });
});

describe("thread rendering for approval continuation states", () => {
  const approvalTail: AssistantMessage[] = [
    {
      id: "message-approval",
      role: "assistant",
      schema_version: 1,
      created_at: new Date(TEST_NOW).toISOString(),
      blocks: [
        {
          type: "approval_card",
          block_id: "approval-1",
          approval_request_id: "request-1",
          body: "Post the summary to #payments-oncall.",
          service_slug: "lark-bot",
          agent_key_prefix: "nyxid_ag_...7f3d",
          approval_mode: "per_request",
          grant_duration_sec: null,
          expires_at: new Date(TEST_NOW + 60_000).toISOString(),
          decision: "approved",
          decision_channel: "web",
        },
      ],
    },
  ];

  it("a printed approval decision does not show an empty-turn error", async () => {
    render(
      <ChatThread
        messages={approvalTail}
        turnEnded
        turnPrinted
        onDecideApproval={() => Promise.resolve()}
      />,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(800);
    });
    expect(
      screen.queryByText(
        "Sorry, there seems to be an error with the request for now.",
      ),
    ).not.toBeInTheDocument();
  });

  it("an approval continuation's pre-status gap shows its thinking state (NYX-3 / hypothesis P1-d)", () => {
    // Page inputs for the gap between clicking Approve and the continuation's
    // first turn.status: active=false, episode open, and an assistant-owned
    // approval card at the tail. The open episode keeps thinking visible.
    render(
      <ChatThread
        messages={approvalTail}
        thinking
        streaming={false}
        turnEnded={false}
        turnPrinted={false}
        onDecideApproval={() => Promise.resolve()}
      />,
    );

    expect(document.querySelector("[data-streaming-dots]")).not.toBeNull();
    expect(document.querySelector("[data-assistant-halo]")).not.toBeNull();
  });
});
