import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ChatProgressTimeoutError,
  STREAM_PROGRESS_TIMEOUT_MS,
  useAssistantChat,
} from "@/hooks/use-assistant-chat";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const OTHER_ACTOR_ID = "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae";

const META = {
  id: ACTOR_ID,
  title: "Saved chat",
  createdAt: "2026-08-24T00:00:00Z",
  updatedAt: "2026-08-24T00:01:00Z",
  messageCount: 2,
};

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function sse(frames: readonly unknown[], keepOpen = false): Response {
  const encoder = new TextEncoder();
  return new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        for (const frame of frames) {
          controller.enqueue(encoder.encode(`data: ${JSON.stringify(frame)}\n\n`));
        }
        if (!keepOpen) controller.close();
      },
    }),
    { status: 200, headers: { "Content-Type": "text/event-stream" } },
  );
}

function currentState(stateVersion = 3): unknown {
  return {
    status: "current",
    stateVersion,
    snapshot: {
      actorId: ACTOR_ID,
      scopeId: "scope-alpha",
      stateVersion,
      progressSequence: stateVersion,
      activeTurn: null,
      latestTurn: { turnId: "turn-saved", status: "completed" },
      recentTerminalTurns: [],
      activeTask: null,
      pendingInput: null,
      pendingApproval: null,
      pendingActions: [],
      recentActions: [],
    },
  };
}

function installDefaultMock(
  postResponse: (body: Record<string, unknown>) => Response = () =>
    sse([
      { runStarted: { actorId: ACTOR_ID, runId: "turn-live" } },
      { textMessageContent: { messageId: "message-live", delta: "Hello" } },
      { textMessageEnd: { messageId: "message-live", message: "Hello" } },
      { runFinished: { runId: "turn-live", result: {} } },
    ]),
) {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint === "/assistant/conversations") {
      return json({ conversations: [META] });
    }
    if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
      return json({
        messages: [
          {
            id: "user-saved",
            turnId: "turn-saved",
            role: "user",
            content: "Saved prompt",
            timestamp: 1,
            status: "completed",
          },
          {
            id: "assistant-saved",
            turnId: "turn-saved",
            role: "assistant",
            content: "Saved answer",
            timestamp: 2,
            status: "completed",
          },
        ],
        stateVersion: 3,
        projectionStatus: "current",
      });
    }
    if (endpoint.startsWith(`/assistant/conversations/${ACTOR_ID}/state`)) {
      return json(currentState());
    }
    if (endpoint === "/assistant/chat") {
      return postResponse(JSON.parse(String(init.body)) as Record<string, unknown>);
    }
    throw new Error(`Unhandled assistant mock request: ${endpoint}`);
  };
}

describe("useAssistantChat", () => {
  beforeEach(() => {
    useAssistantWireLogStore.setState({
      featureEnabled: false,
      captureEnabled: false,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
    installDefaultMock();
  });

  afterEach(() => {
    globalThis.__nyxidAssistantHttpMock = undefined;
    vi.useRealTimers();
  });

  it("restores transcript and actor state in one selection flow", async () => {
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    expect(result.current.session).toMatchObject({
      conversationId: ACTOR_ID,
      expectedTurnCount: 2,
      latestTurnId: "turn-saved",
      title: "Saved chat",
      status: "completed_text",
    });
    expect(result.current.session?.messages[1]).toMatchObject({
      content: "Saved answer",
      status: "complete",
      turnId: "turn-saved",
    });
    expect(result.current.projection?.stateVersion).toBe(3);
  });

  it("retries reload_required state without a cursor", async () => {
    const statePaths: string[] = [];
    let stateRead = 0;
    const original = globalThis.__nyxidAssistantHttpMock;
    globalThis.__nyxidAssistantHttpMock = (request) => {
      if (request.endpoint.includes("/state")) {
        statePaths.push(request.endpoint);
        stateRead += 1;
        return json(stateRead === 1 ? { status: "reload_required" } : currentState(4));
      }
      return original?.(request);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() => expect(result.current.projection?.stateVersion).toBe(4));
    expect(statePaths).toEqual([
      `/assistant/conversations/${ACTOR_ID}/state`,
      `/assistant/conversations/${ACTOR_ID}/state`,
    ]);
  });

  it("adopts RUN_STARTED identity and settles canonical output", async () => {
    const adopted = vi.fn();
    const { result } = renderHook(() =>
      useAssistantChat({ onConversationAdopted: adopted }),
    );
    await waitFor(() => expect(result.current.listLoading).toBe(false));
    await act(async () => result.current.send("  First prompt  "));
    expect(adopted).toHaveBeenCalledWith(ACTOR_ID);
    expect(result.current.session).toMatchObject({
      conversationId: ACTOR_ID,
      latestTurnId: "turn-live",
      expectedTurnCount: 1,
      status: "completed_text",
      title: "First prompt",
      runtime: { actorId: ACTOR_ID, runId: "turn-live" },
    });
    expect(result.current.session?.messages.at(-1)).toMatchObject({
      role: "assistant",
      content: "Hello",
      status: "complete",
    });
  });

  it("rejects a changed authoritative conversation identity", async () => {
    installDefaultMock(() =>
      sse([{ runStarted: { actorId: OTHER_ACTOR_ID, runId: "turn-wrong" } }]),
    );
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    let rejection: unknown;
    await act(async () => {
      try {
        await result.current.send("Continue");
      } catch (error) {
        rejection = error;
      }
    });
    expect(rejection).toEqual(
      expect.objectContaining({
        message: expect.stringContaining("different conversation identity"),
      }),
    );
    expect(result.current.session?.status).toBe("error");
  });

  it("settles a server RUN_STOPPED quietly", async () => {
    installDefaultMock(() =>
      sse([
        { runStarted: { actorId: ACTOR_ID, runId: "turn-stopped" } },
        { runStopped: { runId: "turn-stopped", reason: "reader_cancelled" } },
      ]),
    );
    const { result } = renderHook(() => useAssistantChat({}));
    await waitFor(() => expect(result.current.listLoading).toBe(false));
    await act(async () => result.current.send("Stop this"));
    expect(result.current.session?.status).toBe("stopped");
    expect(result.current.session?.messages.at(-1)).toMatchObject({
      status: "complete",
      error: undefined,
    });
  });

  it("settles a local reader abort quietly", async () => {
    installDefaultMock((body) =>
      body.type === "task.stop"
        ? json({ accepted: true }, 202)
        : sse(
            [{ runStarted: { actorId: ACTOR_ID, runId: "turn-open" } }],
            true,
          ),
    );
    const { result } = renderHook(() => useAssistantChat({}));
    await waitFor(() => expect(result.current.listLoading).toBe(false));
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Watch this");
    });
    await waitFor(() => expect(result.current.session?.latestTurnId).toBe("turn-open"));
    await act(async () => result.current.stop());
    await act(async () => sendPromise!);
    expect(result.current.session?.status).toBe("stopped");
    expect(result.current.session?.messages.at(-1)?.error).toBeUndefined();
  });

  it("expires a keepalive-only stream and issues a best-effort stop", async () => {
    vi.useFakeTimers();
    const commands: Record<string, unknown>[] = [];
    installDefaultMock((body) => {
      commands.push(body);
      return body.type === "task.stop"
        ? json({ accepted: true }, 202)
        : sse(
            [
              { runStarted: { actorId: ACTOR_ID, runId: "turn-timeout" } },
              {
                custom: {
                  name: "aevatar.nyxid_chat.keepalive",
                  payload: { turnId: "turn-timeout" },
                },
              },
            ],
            true,
          );
    });
    const { result } = renderHook(() => useAssistantChat({}));
    await act(async () => vi.advanceTimersByTimeAsync(0));
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Wait forever");
    });
    const observed = sendPromise!.then(
      () => undefined,
      (error: unknown) => error,
    );
    await act(async () => vi.advanceTimersByTimeAsync(0));
    await act(async () => vi.advanceTimersByTimeAsync(STREAM_PROGRESS_TIMEOUT_MS));
    await expect(observed).resolves.toBeInstanceOf(ChatProgressTimeoutError);
    expect(commands.some((command) => command.type === "task.stop")).toBe(true);
    expect(result.current.session?.status).toBe("error");
  });
});
