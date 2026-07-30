import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
} from "@/lib/assistant/errors";
import {
  assistantTransport,
  resetAssistantTransport,
} from "@/lib/assistant/transport";
import type { Conversation, ConversationHistory } from "@/types/assistant";
import {
  assistantKeys,
  describeHistoryError,
  describeSendFailure,
  describeTransportError,
  useAssistantTurn,
  useCancelTurn,
  useConversation,
  useCreateConversation,
  useDecideApproval,
  useSendMessage,
  useTurnEpisode,
  type SentMessage,
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
  resetAssistantTransport(() => TEST_NOW);
  vi.useRealTimers();
});

describe("assistant hooks", () => {
  it("appends a user message and progressively streams the reply", async () => {
    const { queryClient, Wrapper } = createHarness();
    const { result, unmount } = renderHook(
      () => ({
        history: useConversation("conversation-stripe"),
        turn: useAssistantTurn("conversation-stripe"),
        send: useSendMessage("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const initialCount = result.current.history.data?.messages.length ?? 0;

    await act(async () => {
      await result.current.send.mutateAsync("Check the audit trail.");
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.turn.data?.status).toBe("running");
    expect(result.current.history.data?.messages).toHaveLength(
      initialCount + 1,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    const partial = result.current.history.data?.messages.at(-1)?.blocks[0];
    expect(partial?.type).toBe("text");
    if (partial?.type === "text") {
      expect(partial.text.length).toBeGreaterThan(0);
      expect(partial.text).not.toContain("API transport swap");
    }

    await act(async () => {
      // The scripted turn now includes an action-card frame before its
      // terminal event, so allow that additional cadence tick to settle.
      await vi.advanceTimersByTimeAsync(800);
    });
    const completed = result.current.history.data?.messages.at(-1)?.blocks[0];
    expect(result.current.turn.data?.status).toBe("completed");
    expect(completed?.type).toBe("text");
    if (completed?.type === "text") {
      expect(completed.text).toContain("API transport swap");
    }

    unmount();
    queryClient.clear();
  });

  it("opens an episode at send and closes it only when the stream ends", async () => {
    // The episode is what the thread reads to tell "this stream is starting"
    // from "the previous turn's terminal status is still cached", and to know
    // whether THIS stream has printed anything. Neither is answerable from the
    // turn status or the transcript.
    const { queryClient, Wrapper } = createHarness();
    const { result, unmount } = renderHook(
      () => ({
        episode: useTurnEpisode("conversation-stripe"),
        send: useSendMessage("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.episode.data).toBeNull();

    await act(async () => {
      await result.current.send.mutateAsync("Check the audit trail.");
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.episode.data?.open).toBe(true);
    expect(result.current.episode.data?.printed).toBe(false);

    // First characters stream: the dots have to give way from here on.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    expect(result.current.episode.data?.printed).toBe(true);
    expect(result.current.episode.data?.open).toBe(true);

    // Past the end of the scripted turn with room to spare. Don't tighten this
    // to land exactly on the terminal event: the script grows as mock blocks
    // are added (the run and action-card blocks pushed turn.completed from
    // 800ms to 1200ms), and the closing projection settles a tick after it.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(result.current.episode.data?.open).toBe(false);
    expect(result.current.episode.data?.projecting).toBe(false);

    unmount();
    queryClient.clear();
  });

  it("restores the live episode when a concurrent send is rejected", async () => {
    const { queryClient, Wrapper } = createHarness();
    const { result, unmount } = renderHook(
      () => ({
        episode: useTurnEpisode("conversation-stripe"),
        first: useSendMessage("conversation-stripe"),
        second: useSendMessage("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    await act(async () => {
      await result.current.first.mutateAsync("First");
      await vi.advanceTimersByTimeAsync(0);
    });

    // Rejected by the active-turn guard: the losing pump must restore the
    // current stream's episode instead of clearing or replacing it.
    await act(async () => {
      await expect(
        result.current.second.mutateAsync("Second"),
      ).rejects.toBeInstanceOf(AssistantTurnActiveError);
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.episode.data).toMatchObject({
      open: true,
      printed: false,
    });

    unmount();
    queryClient.clear();
  });

  it("auto-creates a conversation for a send with none selected", async () => {
    // The "New chat" empty state has no conversation. The first send must
    // create one and stream into it — never silently no-op (the pre-fix
    // behavior: an internal throw the composer swallowed).
    const { queryClient, Wrapper } = createHarness();
    const { result, unmount } = renderHook(
      () => ({
        send: useSendMessage(undefined),
      }),
      { wrapper: Wrapper },
    );

    let sent: SentMessage | null = null;
    await act(async () => {
      sent = await result.current.send.mutateAsync(
        "Hello from the empty state.",
      );
      await vi.advanceTimersByTimeAsync(0);
    });

    const conversationId = (sent as SentMessage | null)?.conversationId ?? "";
    expect(conversationId).not.toBe("");
    const history = queryClient.getQueryData<ConversationHistory>(
      assistantKeys.history(conversationId),
    );
    expect(history?.messages.at(0)?.role).toBe("user");
    expect(history?.messages.at(0)?.blocks[0]).toMatchObject({
      type: "text",
      text: "Hello from the empty state.",
    });

    // The scripted turn still streams to completion in the new conversation.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    const settled = queryClient.getQueryData<ConversationHistory>(
      assistantKeys.history(conversationId),
    );
    expect(settled?.messages.at(-1)?.role).toBe("assistant");

    unmount();
    queryClient.clear();
  });

  it("shares one auto-created conversation across racing empty-state sends", async () => {
    // Two sends arriving before React commits the disabled/sending state
    // must not allocate two actors: the create is single-flight, and the
    // loser is rejected by the active-turn guard.
    const { queryClient, Wrapper } = createHarness();
    const createSpy = vi.spyOn(assistantTransport, "createConversation");
    const { result, unmount } = renderHook(
      () => ({
        send: useSendMessage(undefined),
      }),
      { wrapper: Wrapper },
    );

    let outcomes: PromiseSettledResult<SentMessage>[] = [];
    await act(async () => {
      const race = Promise.allSettled([
        result.current.send.mutateAsync("First racing send."),
        result.current.send.mutateAsync("Second racing send."),
      ]);
      await vi.advanceTimersByTimeAsync(0);
      outcomes = await race;
    });

    expect(createSpy).toHaveBeenCalledTimes(1);
    const fulfilled = outcomes.filter(
      (outcome) => outcome.status === "fulfilled",
    );
    const rejected = outcomes.filter(
      (outcome) => outcome.status === "rejected",
    );
    expect(fulfilled).toHaveLength(1);
    expect(rejected).toHaveLength(1);
    expect(String(rejected[0]?.reason)).toMatch(/already active/i);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    createSpy.mockRestore();
    unmount();
    queryClient.clear();
  });

  it("shares one conversation between the New chat button and a racing send", async () => {
    // "New chat" navigates optimistically, so the draft thread accepts a
    // send while its actor is still being provisioned. The button's create
    // and the empty-state auto-create must resolve to the SAME actor —
    // otherwise the message streams into an orphan the sidebar never shows.
    const { queryClient, Wrapper } = createHarness();
    const createSpy = vi.spyOn(assistantTransport, "createConversation");
    const { result, unmount } = renderHook(
      () => ({
        create: useCreateConversation(),
        send: useSendMessage(undefined),
      }),
      { wrapper: Wrapper },
    );

    let created: Conversation | null = null;
    let sent: SentMessage | null = null;
    await act(async () => {
      const createPromise = result.current.create.mutateAsync();
      const sendPromise = result.current.send.mutateAsync(
        "Sent mid-provision.",
      );
      await vi.advanceTimersByTimeAsync(0);
      [created, sent] = await Promise.all([createPromise, sendPromise]);
    });

    expect(createSpy).toHaveBeenCalledTimes(1);
    expect((sent as SentMessage | null)?.conversationId).toBe(
      (created as Conversation | null)?.id,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    createSpy.mockRestore();
    unmount();
    queryClient.clear();
  });

  it("toasts when a started stream later fails, since the mutation already resolved", async () => {
    // Pre-SSE rejections and truncated streams surface as a failed
    // `turn.completed` AFTER `mutateAsync` resolved — without a toast the
    // failure would exist only as cached state nothing renders.
    const { queryClient, Wrapper } = createHarness();
    const toastSpy = vi.spyOn(toast, "error");
    const sendSpy = vi
      .spyOn(assistantTransport, "sendMessage")
      .mockImplementation((_conversationId, _content, onEvent) => {
        onEvent({
          cursor: 1,
          event: "turn.status",
          turn_id: "turn-doomed",
          status: "running",
        });
        onEvent({
          cursor: 2,
          event: "turn.completed",
          turn_id: "turn-doomed",
          status: "failed",
          error: { code: "http_502", message: "Aevatar timed out." },
        });
        return { turnId: "turn-doomed", cancel: () => {} };
      });
    const { result, unmount } = renderHook(
      () => ({ send: useSendMessage("conversation-stripe") }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.send.mutateAsync("Doomed message.");
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(toastSpy).toHaveBeenCalledWith(
      "The assistant reply failed",
      expect.objectContaining({ description: "Aevatar timed out." }),
    );

    sendSpy.mockRestore();
    toastSpy.mockRestore();
    unmount();
    queryClient.clear();
  });

  it("coalesces bursty stream frames before projecting them into React", async () => {
    const { queryClient, Wrapper } = createHarness();
    const historySpy = vi.spyOn(assistantTransport, "getHistory");
    let emit: Parameters<typeof assistantTransport.sendMessage>[2] | undefined;
    const sendSpy = vi
      .spyOn(assistantTransport, "sendMessage")
      .mockImplementation((_conversationId, _content, onEvent) => {
        emit = onEvent;
        for (let cursor = 1; cursor <= 250; cursor += 1) {
          onEvent({
            cursor,
            event: "block.delta",
            block_id: "streaming-text",
            text: "x",
          });
        }
        return { turnId: "turn-burst", cancel: () => {} };
      });
    const { result, unmount } = renderHook(
      () => ({ send: useSendMessage("conversation-stripe") }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.send.mutateAsync("Bursty response.");
    });
    // Ignore the send mutation's intentional immediate projection. The 250
    // stream callbacks above must produce only one additional projection.
    historySpy.mockClear();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(49);
    });
    expect(historySpy).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(historySpy).toHaveBeenCalledTimes(1);

    act(() => {
      emit?.({
        cursor: 251,
        event: "block.delta",
        block_id: "streaming-text",
        text: "final",
      });
      emit?.({
        cursor: 252,
        event: "turn.completed",
        turn_id: "turn-burst",
        status: "completed",
        error: null,
      });
    });
    // Terminal events bypass the interval so Stop/send state cannot linger.
    expect(historySpy).toHaveBeenCalledTimes(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    expect(historySpy).toHaveBeenCalledTimes(2);

    sendSpy.mockRestore();
    historySpy.mockRestore();
    unmount();
    queryClient.clear();
  });

  it("rejects a concurrent send without disturbing the active turn", async () => {
    const { queryClient, Wrapper } = createHarness();
    const { result, unmount } = renderHook(
      () => ({
        history: useConversation("conversation-stripe"),
        turn: useAssistantTurn("conversation-stripe"),
        send: useSendMessage("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.send.mutateAsync("First message.");
      await vi.advanceTimersByTimeAsync(150);
    });
    expect(result.current.turn.data?.status).toBe("running");

    // Second send while the turn is active must reject...
    await act(async () => {
      await expect(
        result.current.send.mutateAsync("Second message."),
      ).rejects.toThrow(/already active/i);
    });

    // ...and the first turn still streams to completion afterwards.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(result.current.turn.data?.status).toBe("completed");
    const completed = result.current.history.data?.messages.at(-1)?.blocks[0];
    expect(completed?.type === "text" ? completed.text : "").toContain(
      "API transport swap",
    );

    unmount();
    queryClient.clear();
  });

  it("cancels a turn mid-stream and prevents later timer writes", async () => {
    const { queryClient, Wrapper } = createHarness();
    const { result, unmount } = renderHook(
      () => ({
        history: useConversation("conversation-stripe"),
        turn: useAssistantTurn("conversation-stripe"),
        send: useSendMessage("conversation-stripe"),
        cancel: useCancelTurn("conversation-stripe"),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await result.current.send.mutateAsync("Start then stop.");
      await vi.advanceTimersByTimeAsync(350);
    });
    await act(async () => {
      await result.current.cancel.mutateAsync();
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.turn.data?.status).toBe("cancelled");
    const cancelledText =
      result.current.history.data?.messages.at(-1)?.blocks[0];
    expect(cancelledText?.type).toBe("text");
    const snapshot = cancelledText?.type === "text" ? cancelledText.text : "";

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    const afterTimers = result.current.history.data?.messages.at(-1)?.blocks[0];
    expect(afterTimers?.type === "text" ? afterTimers.text : "").toBe(snapshot);

    unmount();
    queryClient.clear();
  });

  it.each([
    {
      approved: true,
      decision: "approved",
      runState: "completed",
      stepStatus: "done",
      stepsComplete: 3,
    },
    {
      approved: false,
      decision: "denied",
      runState: "failed",
      stepStatus: "failed",
      stepsComplete: 2,
    },
  ])(
    "projects the $decision approval branch into the card and run",
    async ({ approved, decision, runState, stepStatus, stepsComplete }) => {
      const { queryClient, Wrapper } = createHarness();
      const { result, unmount } = renderHook(
        () => ({
          history: useConversation("conversation-stripe"),
          decide: useDecideApproval("conversation-stripe"),
        }),
        { wrapper: Wrapper },
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
        await result.current.decide.mutateAsync({
          blockId: "approval-stripe-lark",
          approved,
        });
        await vi.advanceTimersByTimeAsync(0);
      });

      const blocks = result.current.history.data?.messages.flatMap(
        (message) => message.blocks,
      );
      const approval = blocks?.find((block) => block.type === "approval_card");
      const run = blocks?.find((block) => block.type === "run");
      expect(
        approval?.type === "approval_card" ? approval.decision : null,
      ).toBe(decision);
      expect(run?.type === "run" ? run.state : null).toBe(runState);
      expect(run?.type === "run" ? run.steps_complete : null).toBe(
        stepsComplete,
      );
      expect(run?.type === "run" ? run.steps.at(-1)?.status : null).toBe(
        stepStatus,
      );

      unmount();
      queryClient.clear();
    },
  );
});

describe("describeTransportError", () => {
  // A downstream 401 from `/proxy/s/aevatar` means aevatar rejected the
  // forwarded identity, NOT that the NyxID session died. The copy must say
  // "you are still signed in" — the whole point of the fix is that we no
  // longer bounce to /login here.
  it("explains a downstream 401 without implying the session died", () => {
    const { message, description } = describeTransportError(
      new ApiError(401, {
        error: "unknown_error",
        error_code: -1,
        message: "Request failed with status 401",
      }),
    );
    expect(message).toBe("Assistant chat is unavailable");
    expect(description).toContain("still signed in");
  });

  it("falls back to a generic message for non-401 failures", () => {
    const { message } = describeTransportError(new Error("network down"));
    expect(message).toBe("Could not load your chats");
  });
});

describe("describeSendFailure", () => {
  it("explains a downstream 401/403 without implying the session died", () => {
    const { message, description } = describeSendFailure(
      new ApiError(401, {
        error: "unauthorized",
        error_code: 1001,
        message: "Unauthorized",
      }),
    );
    expect(message).toBe("Message not sent");
    expect(description).toContain("still signed in");
  });

  it("asks the user to wait when a turn is already active", () => {
    const { description } = describeSendFailure(new AssistantTurnActiveError());
    expect(description).toContain("current reply");
  });

  it("surfaces the underlying error message otherwise", () => {
    const { description } = describeSendFailure(
      new Error("Aevatar did not return a conversation id."),
    );
    expect(description).toBe("Aevatar did not return a conversation id.");
  });
});

describe("conversation not-found resolution", () => {
  it("does not retry a confirmed typed not-found transcript", async () => {
    const { queryClient, Wrapper } = createHarness();
    const historySpy = vi
      .spyOn(assistantTransport, "getHistory")
      .mockRejectedValue(new AssistantConversationNotFoundError());
    const { result, unmount } = renderHook(
      () => useConversation("nyxid-pending-lost-after-reload"),
      { wrapper: Wrapper },
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });

    expect(result.current.error).toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
    expect(historySpy).toHaveBeenCalledTimes(1);

    historySpy.mockRestore();
    unmount();
    queryClient.clear();
  });
});

describe("a failed transcript read never blocks the send", () => {
  // The send path warms the cache before and after the stream starts. Those
  // two reads used to share one `Promise.all`, so a rejected transcript read
  // took the projection — and the send — down with it, and the stream POST
  // was never issued. Reading history and sending a message target different
  // upstream surfaces; one must not gate the other.
  it("still streams the turn when getHistory rejects", async () => {
    const { Wrapper } = createHarness();
    vi.spyOn(assistantTransport, "getHistory").mockRejectedValue(
      new ApiError(404, {
        error: "not_found",
        error_code: -1,
        message: "Not Found",
      }),
    );
    const sendSpy = vi.spyOn(assistantTransport, "sendMessage");
    // The file has no global mock reset; spies accumulate across tests.
    sendSpy.mockClear();

    const { result, unmount } = renderHook(
      () => useSendMessage("conversation-stripe"),
      { wrapper: Wrapper },
    );

    let sent: SentMessage | null = null;
    await act(async () => {
      const pending = result.current.mutateAsync("Send me anyway.");
      await vi.advanceTimersByTimeAsync(0);
      sent = await pending;
    });

    expect(sendSpy).toHaveBeenCalledTimes(1);
    expect((sent as SentMessage | null)?.conversationId).toBe(
      "conversation-stripe",
    );
    unmount();
  });

  it("still streams from the draft empty state, where the read runs FIRST", async () => {
    // The exact prod failure: "New chat" -> first send -> the conversation is
    // allocated, then the cache is warmed, then the stream POST fires. With
    // the read gating the projection, the flow died between allocate and
    // stream and no `:stream` request was ever issued.
    const { Wrapper } = createHarness();
    vi.spyOn(assistantTransport, "getHistory").mockRejectedValue(
      new ApiError(404, {
        error: "not_found",
        error_code: -1,
        message: "Not Found",
      }),
    );
    const sendSpy = vi.spyOn(assistantTransport, "sendMessage");
    // The file has no global mock reset; spies accumulate across tests.
    sendSpy.mockClear();

    const { result, unmount } = renderHook(() => useSendMessage(undefined), {
      wrapper: Wrapper,
    });

    let sent: SentMessage | null = null;
    await act(async () => {
      const pending = result.current.mutateAsync("First message.");
      await vi.advanceTimersByTimeAsync(0);
      sent = await pending;
    });

    expect(sendSpy).toHaveBeenCalledTimes(1);
    expect((sent as SentMessage | null)?.conversationId).toBeTruthy();
    unmount();
  });
});

describe("describeHistoryError", () => {
  it("does not call a not-yet-materialized transcript a failure", () => {
    expect(
      describeHistoryError(
        new ApiError(404, {
          error: "not_found",
          error_code: -1,
          message: "Not Found",
        }),
      ),
    ).toContain("no saved transcript yet");
  });

  it("names an auth rejection as the backend's, not the session's", () => {
    expect(
      describeHistoryError(
        new ApiError(403, {
          error: "forbidden",
          error_code: -1,
          message: "Forbidden",
        }),
      ),
    ).toContain("chat backend rejected");
  });

  it("falls back to a plain message for anything else", () => {
    expect(describeHistoryError(new Error("boom"))).toContain(
      "Could not load earlier messages",
    );
  });
});
