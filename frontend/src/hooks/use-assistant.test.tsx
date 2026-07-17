import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { resetAssistantTransport } from "@/lib/assistant/transport";
import {
  describeTransportError,
  useAssistantTurn,
  useCancelTurn,
  useConversation,
  useDecideApproval,
  useSendMessage,
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
      await vi.advanceTimersByTimeAsync(700);
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
