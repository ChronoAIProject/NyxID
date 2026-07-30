/**
 * AUDIT probes — chat-flow audit, docs/chat-flow-audit.md.
 *
 * These are CHARACTERIZATION tests: each pins down a defect's mechanism
 * empirically and PASSES by asserting the current (defective) behavior, so
 * the suite stays green while the defects are open. Each carries the defect
 * id from the audit report; when a defect is fixed, flip the marked
 * assertion(s) to the desired behavior and move the test into the main
 * suite (or delete it in favor of a real regression test).
 *
 * They drive the REAL hooks against the REAL mock transport (plus targeted
 * spies where the defect lives in the live transport's event vocabulary).
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
  delete (
    globalThis as { __assistantMockFaults?: unknown }
  ).__assistantMockFaults;
  vi.restoreAllMocks();
  resetAssistantTransport(() => TEST_NOW);
  vi.useRealTimers();
});

describe("AUDIT: episode-slot ownership (NYX-2 / hypothesis P1-a)", () => {
  it("a rejected concurrent send nulls the LIVE stream's episode; with no further events it stays null", async () => {
    // A stream that produced no events yet (the real transport before its
    // response headers arrive). With events flowing, the null still happens
    // but is re-overwritten within ~100 ms by the live pump's next event or
    // projection finalizer — itself proof that the slot has no owner: any
    // pump, superseded or not, writes it unconditionally.
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

    // DEFECT (NYX-2): the LOSER's cleanup wiped the WINNER's episode. The
    // page now believes no stream ran — the thinking indicator drops, and a
    // stream that later closes empty can no longer be reported as an error.
    expect(result.current.episode.data).toBeNull();
  });
});

describe("AUDIT: approval leaves the episode open forever (NYX-5 / hypothesis P2-f)", () => {
  it("deciding an approval on the mock transport opens an episode nothing will ever close", async () => {
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

    // DEFECT (NYX-5): the pump is constructed (episode opened) before the
    // transport answers; the mock's decideApproval settles the card and
    // returns null WITHOUT emitting a single event, so nothing ever closes
    // the episode.
    expect(result.current.episode.data).toEqual({
      open: true,
      printed: false,
      projecting: false,
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(result.current.episode.data?.open).toBe(true);
  });
});

describe("AUDIT: approval decision events do not count as printed (NYX-4 / hypothesis P1-c)", () => {
  it("a JSON-ack approval — block.updated patches then turn.completed — closes the episode printed:false", async () => {
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

    // The approval SUCCEEDED and the card visibly flipped via block.updated,
    // yet the episode closes as "printed nothing":
    expect(result.current.turn.data?.status).toBe("completed");
    // DEFECT (NYX-4): eventPrintsContent ignores block.updated, so a turn
    // whose only presentation events are approval patches reads as empty.
    expect(result.current.episode.data).toMatchObject({
      open: false,
      printed: false,
    });
    // pages/assistant.tsx will therefore compute turnEnded=true,
    // turnPrinted=false — the combination ChatThread reports as the red
    // "Sorry, there seems to be an error" row (proven in the render probe
    // below).
  });
});

describe("AUDIT: what the thread renders for the probed states (NYX-4 consequence)", () => {
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

  it("turnEnded + printed:false over an approved card shows the false error after the grace period", async () => {
    render(
      <ChatThread
        messages={approvalTail}
        turnEnded
        turnPrinted={false}
        onDecideApproval={() => Promise.resolve()}
      />,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(800);
    });
    // DEFECT (NYX-4, user-visible half): the reader approved, the card
    // flipped, the continuation acked — and the chat calls it an error.
    expect(
      screen.getByText(
        "Sorry, there seems to be an error with the request for now.",
      ),
    ).toBeInTheDocument();
  });

  it("an approval continuation's pre-status gap shows NO loading indicator at all (NYX-3 / hypothesis P1-d)", () => {
    // Probed page inputs for the gap between clicking Approve and the
    // continuation's first turn.status: active=false (turn cache still
    // holds the prior terminal), episode open (pump constructed), tail is
    // the assistant-owned approval card. The page then passes thinking=false
    // (tail IS assistant) and streaming=false (turn not active).
    render(
      <ChatThread
        messages={approvalTail}
        thinking={false}
        streaming={false}
        turnEnded={false}
        turnPrinted={false}
        onDecideApproval={() => Promise.resolve()}
      />,
    );

    // DEFECT (NYX-3): nothing on screen says the assistant is working.
    expect(document.querySelector("[data-streaming-dots]")).toBeNull();
    expect(document.querySelector("[data-assistant-halo]")).toBeNull();
  });
});
