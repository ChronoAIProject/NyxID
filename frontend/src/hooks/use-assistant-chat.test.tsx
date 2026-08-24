import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ChatProgressTimeoutError,
  ChatStartTimeoutError,
  STREAM_PROGRESS_TIMEOUT_MS,
  STREAM_START_DEADLINE_MS,
  useAssistantChat,
} from "@/hooks/use-assistant-chat";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const OTHER_ACTOR_ID = "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae";
const LEGACY_ID = "chatc-8bd999c402fb37d60cdcd81e3b78cfd";

const META = {
  id: ACTOR_ID,
  title: "Saved chat",
  createdAt: "2026-08-24T00:00:00Z",
  updatedAt: "2026-08-24T00:01:00Z",
  messageCount: 2,
};

const OTHER_META = {
  ...META,
  id: OTHER_ACTOR_ID,
  title: "Other saved chat",
  messageCount: 1,
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
          controller.enqueue(
            encoder.encode(`data: ${JSON.stringify(frame)}\n\n`),
          );
        }
        if (!keepOpen) controller.close();
      },
    }),
    { status: 200, headers: { "Content-Type": "text/event-stream" } },
  );
}

function controlledSse() {
  const encoder = new TextEncoder();
  let controller: ReadableStreamDefaultController<Uint8Array>;
  const response = new Response(
    new ReadableStream<Uint8Array>({
      start(next) {
        controller = next;
      },
    }),
    { status: 200, headers: { "Content-Type": "text/event-stream" } },
  );
  return {
    response,
    push(frame: unknown) {
      controller.enqueue(encoder.encode(`data: ${JSON.stringify(frame)}\n\n`));
    },
    close() {
      controller.close();
    },
  };
}

function responsePendingUntilAbort(
  signal: AbortSignal | null | undefined,
): Promise<Response> {
  return new Promise((_, reject) => {
    const rejectWithAbort = () =>
      reject(signal?.reason ?? new DOMException("Aborted", "AbortError"));
    if (signal?.aborted) rejectWithAbort();
    else signal?.addEventListener("abort", rejectWithAbort, { once: true });
  });
}

function currentState(
  stateVersion = 3,
  actorId = ACTOR_ID,
  snapshotOverrides: Record<string, unknown> = {},
): unknown {
  return {
    status: "current",
    stateVersion,
    snapshot: {
      actorId,
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
      ...snapshotOverrides,
    },
  };
}

function storedDetail(
  actorId: string,
  title: string,
  messageCount = 2,
): Record<string, unknown> {
  const messages = [
    {
      id: `${actorId}-user-saved`,
      turnId: `${actorId}-turn-saved`,
      role: "user",
      content: `${title} prompt`,
      timestamp: 1,
      status: "completed",
    },
    {
      id: `${actorId}-assistant-saved`,
      turnId: `${actorId}-turn-saved`,
      role: "assistant",
      content: `${title} answer`,
      timestamp: 2,
      status: "completed",
    },
  ];
  return {
    messages: messages.slice(0, messageCount),
    stateVersion: 3,
    projectionStatus: "current",
  };
}

function installSwitchingMock(stream: ReturnType<typeof controlledSse>) {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (endpoint === "/assistant/conversations") {
      return json({ conversations: [META, OTHER_META] });
    }
    if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
      return json(storedDetail(ACTOR_ID, "Saved chat"));
    }
    if (endpoint === `/assistant/conversations/${OTHER_ACTOR_ID}`) {
      return json(storedDetail(OTHER_ACTOR_ID, "Other saved chat", 1));
    }
    if (endpoint.startsWith(`/assistant/conversations/${ACTOR_ID}/state`)) {
      return json(currentState(3, ACTOR_ID));
    }
    if (
      endpoint.startsWith(`/assistant/conversations/${OTHER_ACTOR_ID}/state`)
    ) {
      return json(currentState(3, OTHER_ACTOR_ID));
    }
    if (endpoint === "/assistant/chat") return stream.response;
    throw new Error(`Unhandled assistant mock request: ${endpoint}`);
  };
}

function actorControlState(stateVersion = 7): unknown {
  return currentState(stateVersion, ACTOR_ID, {
    activeTurn: {
      turnId: "turn-active",
      taskId: "task-active",
      status: "active",
    },
    latestTurn: null,
    activeTask: {
      schemaVersion: 4,
      actorId: ACTOR_ID,
      taskId: "task-active",
      turnId: "turn-active",
      planId: "plan-active",
      planRevision: 2,
      planRevisions: [],
      title: "Apply the requested change",
      status: "active",
      activeStepId: "step-active",
      gate: {
        mode: "confirm",
        status: "pending",
        requestId: "plan-request",
        taskId: "task-active",
        planId: "plan-active",
        planRevision: 2,
      },
      steps: [
        {
          stepId: "step-active",
          order: 1,
          kind: "tool",
          status: "failed",
          required: true,
          description: "Apply change",
          source: { tool: { toolName: "apply_change" } },
          mayChangeExternalState: true,
          externalEffect: "not_applied",
          availableActions: { retry: true, skip: true, stop: true },
          dependsOn: [],
          substeps: [],
          operation: {
            conversationActorId: ACTOR_ID,
            turnId: "turn-active",
            taskId: "task-active",
            stepId: "step-active",
            operationId: "operation-active",
            operationGeneration: 4,
            phase: "failed",
          },
        },
      ],
    },
    pendingInput: {
      requestId: "input-active",
      prompt: "Choose a region",
      options: [
        { optionId: "option-sg", label: "Singapore" },
        { optionId: "option-fra", label: "Frankfurt" },
      ],
      allowFreeText: false,
      multiSelect: false,
    },
    pendingApproval: {
      approvalRequestId: "approval-active",
      toolName: "apply_change",
    },
  });
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
      return postResponse(
        JSON.parse(String(init.body)) as Record<string, unknown>,
      );
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
        return json(
          stateRead === 1 ? { status: "reload_required" } : currentState(4),
        );
      }
      return original?.(request);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() =>
      expect(result.current.projection?.stateVersion).toBe(4),
    );
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

  it("rejects a changed pre-start identity and restores the settled session", async () => {
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
    expect(result.current.session?.status).toBe("completed_text");
    expect(result.current.session?.messages).toHaveLength(2);
    expect(result.current.session?.messages.at(-1)?.content).toBe(
      "Saved answer",
    );
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
    await waitFor(() =>
      expect(result.current.session?.latestTurnId).toBe("turn-open"),
    );
    await act(async () => result.current.stop());
    await act(async () => sendPromise!);
    expect(result.current.session?.status).toBe("stopped");
    expect(result.current.session?.messages.at(-1)?.error).toBeUndefined();
  });

  it("stops locally while response headers are still pending", async () => {
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [] });
      }
      if (endpoint === "/assistant/chat") {
        return responsePendingUntilAbort(init.signal);
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() => useAssistantChat({}));
    await waitFor(() => expect(result.current.listLoading).toBe(false));
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Stop before headers");
    });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    await act(async () => result.current.stop());
    await act(async () => sendPromise!);

    expect(result.current.isStreaming).toBe(false);
    expect(result.current.session?.status).toBe("stopped");
    expect(result.current.session?.messages.at(-1)?.error).toBeUndefined();
  });

  it("rejects a silent pre-start request at the 30 second deadline", async () => {
    vi.useFakeTimers();
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [] });
      }
      if (endpoint === "/assistant/chat") {
        return responsePendingUntilAbort(init.signal);
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() => useAssistantChat({}));
    await act(async () => vi.advanceTimersByTimeAsync(0));
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Wait for a reply");
    });
    const observed = sendPromise!.then(
      () => undefined,
      (error: unknown) => error,
    );

    await act(async () =>
      vi.advanceTimersByTimeAsync(STREAM_START_DEADLINE_MS),
    );

    await expect(observed).resolves.toBeInstanceOf(ChatStartTimeoutError);
    expect(result.current.isStreaming).toBe(false);
    expect(result.current.session?.status).toBe("error");
    expect(result.current.session?.messages.at(-1)?.error).toBe(
      new ChatStartTimeoutError().message,
    );
    expect(result.current.session?.messages).toHaveLength(2);
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
    await act(async () =>
      vi.advanceTimersByTimeAsync(STREAM_PROGRESS_TIMEOUT_MS),
    );
    await expect(observed).resolves.toBeUndefined();
    expect(commands.some((command) => command.type === "task.stop")).toBe(true);
    expect(result.current.session?.status).toBe("error");
    expect(result.current.session?.messages.at(-1)?.error).toBe(
      new ChatProgressTimeoutError().message,
    );
  });

  it("rejects an HTTP failure before RUN_STARTED and removes the optimistic turn", async () => {
    installDefaultMock(() =>
      json(
        {
          error: "assistant_unavailable",
          error_code: 9100,
          message: "Assistant unavailable",
        },
        503,
      ),
    );
    const { result } = renderHook(() => useAssistantChat({}));
    await waitFor(() => expect(result.current.listLoading).toBe(false));

    await expect(
      act(async () => result.current.send("Keep this draft")),
    ).rejects.toMatchObject({
      message: "Assistant unavailable",
      status: 503,
      code: 9100,
    });
    expect(result.current.session).toMatchObject({
      status: "draft",
      messages: [],
    });
  });

  it("restores a legacy transcript when actor state is not found and refuses writes", async () => {
    const posts: Record<string, unknown>[] = [];
    const legacyMeta = { ...META, id: LEGACY_ID, title: "Legacy transcript" };
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [legacyMeta] });
      }
      if (endpoint === `/assistant/conversations/${LEGACY_ID}`) {
        return json(storedDetail(LEGACY_ID, "Legacy transcript"));
      }
      if (endpoint === `/assistant/conversations/${LEGACY_ID}/state`) {
        return json({ message: "Actor state unavailable" }, 404);
      }
      if (endpoint === "/assistant/chat") {
        posts.push(JSON.parse(String(init.body)) as Record<string, unknown>);
        return json({ accepted: true }, 202);
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: LEGACY_ID }),
    );

    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    expect(result.current.projection).toMatchObject({
      actorId: LEGACY_ID,
      stateVersion: 0,
      pendingInput: null,
    });
    await act(async () => result.current.send("Do not send"));
    await act(async () => result.current.steer("Do not steer"));
    expect(posts).toEqual([]);
  });

  it("keeps a known transcript 404 as a usable no-transcript placeholder", async () => {
    const commands: Record<string, unknown>[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [META] });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
        return json({ message: "Transcript not materialized" }, 404);
      }
      if (endpoint.startsWith(`/assistant/conversations/${ACTOR_ID}/state`)) {
        return json({ status: "not_found" });
      }
      if (endpoint === "/assistant/chat") {
        const body = JSON.parse(String(init.body)) as Record<string, unknown>;
        commands.push(body);
        return sse([
          { runStarted: { actorId: ACTOR_ID, runId: "turn-after-404" } },
          {
            textMessageEnd: { messageId: "answer-after-404", message: "Ready" },
          },
          { runFinished: { runId: "turn-after-404", result: {} } },
        ]);
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );

    await waitFor(() =>
      expect(result.current.detailState.status).toBe("missing"),
    );
    expect(result.current.session).toMatchObject({
      conversationId: ACTOR_ID,
      messages: [],
      title: "Saved chat",
    });
    await act(async () => result.current.send("Continue anyway"));
    expect(commands).toContainEqual(
      expect.objectContaining({ type: "text", prompt: "Continue anyway" }),
    );
    expect(result.current.session?.status).toBe("completed_text");
  });

  it.each([
    [404, "missing"],
    [503, "error"],
  ] as const)(
    "retries a settled %i transcript failure when the conversation is reselected",
    async (status, detailStatus) => {
      let actorReads = 0;
      globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
        if (endpoint === "/assistant/conversations") {
          return json({ conversations: [META, OTHER_META] });
        }
        if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
          actorReads += 1;
          return actorReads === 1
            ? json({ message: "Transcript unavailable" }, status)
            : json(storedDetail(ACTOR_ID, "Saved chat"));
        }
        if (endpoint === `/assistant/conversations/${OTHER_ACTOR_ID}`) {
          return json(storedDetail(OTHER_ACTOR_ID, "Other saved chat", 1));
        }
        if (endpoint.startsWith(`/assistant/conversations/${ACTOR_ID}/state`)) {
          return json(currentState(3, ACTOR_ID));
        }
        if (
          endpoint.startsWith(
            `/assistant/conversations/${OTHER_ACTOR_ID}/state`,
          )
        ) {
          return json(currentState(3, OTHER_ACTOR_ID));
        }
        throw new Error(`Unhandled assistant mock request: ${endpoint}`);
      };
      const { result, rerender } = renderHook(
        ({ conversationId }: { conversationId: string }) =>
          useAssistantChat({ selectedConversationId: conversationId }),
        { initialProps: { conversationId: ACTOR_ID } },
      );
      await waitFor(() =>
        expect(result.current.detailState.status).toBe(detailStatus),
      );

      rerender({ conversationId: OTHER_ACTOR_ID });
      await waitFor(() =>
        expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID),
      );
      rerender({ conversationId: ACTOR_ID });
      await waitFor(() =>
        expect(result.current.session?.messages).toHaveLength(2),
      );

      expect(actorReads).toBe(2);
      expect(result.current.detailState.status).toBe("idle");
    },
  );

  it("repairs an index-absent transcript 404 to New chat", async () => {
    const missing = vi.fn();
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [] });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
        return json({ message: "Conversation not found" }, 404);
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}/state`) {
        return json({ status: "not_found" });
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({
        selectedConversationId: ACTOR_ID,
        onConversationMissing: missing,
      }),
    );

    await waitFor(() => expect(missing).toHaveBeenCalledWith(ACTOR_ID));
    expect(result.current.session).toMatchObject({
      status: "draft",
      title: "New chat",
      messages: [],
    });
  });

  it("bounds empty pending transcript rereads and leaves a nonblocking notice", async () => {
    vi.useFakeTimers();
    let transcriptReads = 0;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [META] });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
        transcriptReads += 1;
        return json({
          messages: [],
          stateVersion: 0,
          projectionStatus: "pending",
        });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}/state`) {
        return json({ status: "not_found" });
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );

    await act(async () => vi.advanceTimersByTimeAsync(0));
    expect(result.current.session).toMatchObject({
      conversationId: ACTOR_ID,
      expectedTurnCount: 2,
      title: "Saved chat",
    });
    await act(async () => vi.advanceTimersByTimeAsync(3_750));
    expect(transcriptReads).toBe(5);
    expect(result.current.detailState.status).toBe("missing");
    expect(result.current.session?.messages).toEqual([]);
  });

  it("uses index title and turn count while a transcript restore is pending", async () => {
    let resolveTranscript: ((response: Response) => void) | undefined;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [META] });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
        return new Promise<Response>((resolve) => {
          resolveTranscript = resolve;
        });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}/state`) {
        return json(currentState());
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );

    await waitFor(() =>
      expect(result.current.detailState.status).toBe("loading"),
    );
    expect(result.current.session).toMatchObject({
      conversationId: ACTOR_ID,
      expectedTurnCount: 2,
      title: "Saved chat",
    });
    await act(async () => {
      resolveTranscript?.(json(storedDetail(ACTOR_ID, "Saved chat")));
    });
    await waitFor(() => expect(result.current.detailState.status).toBe("idle"));
  });

  it("switching away mid-stream leaks nothing; switching back lands in the finished turn", async () => {
    const stream = controlledSse();
    installSwitchingMock(stream);
    const { result, rerender } = renderHook(
      ({ selected }: { selected: string }) =>
        useAssistantChat({ selectedConversationId: selected }),
      { initialProps: { selected: ACTOR_ID } },
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Only for the first chat");
    });
    await act(async () => {
      stream.push({ runStarted: { actorId: ACTOR_ID, runId: "turn-switch" } });
    });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    rerender({ selected: OTHER_ACTOR_ID });
    await waitFor(() =>
      expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID),
    );
    expect(
      result.current.session?.messages.some(
        (message) => message.content === "Only for the first chat",
      ),
    ).toBe(false);

    await act(async () => {
      stream.push({
        textMessageEnd: {
          messageId: "switch-answer",
          message: "Finished away",
        },
      });
      stream.push({ runFinished: { runId: "turn-switch", result: {} } });
      stream.close();
      await sendPromise!;
    });
    expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID);

    rerender({ selected: ACTOR_ID });
    await waitFor(() =>
      expect(result.current.session).toMatchObject({
        conversationId: ACTOR_ID,
        status: "completed_text",
      }),
    );
    expect(result.current.session?.messages.at(-1)?.content).toBe(
      "Finished away",
    );
  });

  it("switching back while the turn still streams resumes its live loading state", async () => {
    const stream = controlledSse();
    installSwitchingMock(stream);
    const { result, rerender } = renderHook(
      ({ selected }: { selected: string }) =>
        useAssistantChat({ selectedConversationId: selected }),
      { initialProps: { selected: ACTOR_ID } },
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Keep streaming here");
    });
    await act(async () => {
      stream.push({
        runStarted: { actorId: ACTOR_ID, runId: "turn-live-switch" },
      });
      stream.push({
        textMessageContent: { messageId: "live-switch", delta: "Partial" },
      });
    });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    rerender({ selected: OTHER_ACTOR_ID });
    await waitFor(() =>
      expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID),
    );
    rerender({ selected: ACTOR_ID });
    await waitFor(() =>
      expect(result.current.session).toMatchObject({
        conversationId: ACTOR_ID,
        latestTurnId: "turn-live-switch",
        status: "streaming",
      }),
    );
    expect(result.current.session?.messages.at(-1)).toMatchObject({
      content: "Partial",
      status: "streaming",
    });

    await act(async () => {
      stream.push({
        textMessageEnd: { messageId: "live-switch", message: "Partial done" },
      });
      stream.push({ runFinished: { runId: "turn-live-switch", result: {} } });
      stream.close();
      await sendPromise!;
    });
  });

  it("an in-flight send's optimistic echo never follows the reader into another chat", async () => {
    const stream = controlledSse();
    installSwitchingMock(stream);
    const { result, rerender } = renderHook(
      ({ selected }: { selected: string }) =>
        useAssistantChat({ selectedConversationId: selected }),
      { initialProps: { selected: ACTOR_ID } },
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Optimistic first-chat echo");
    });
    await waitFor(() =>
      expect(result.current.session?.messages.at(-2)?.content).toBe(
        "Optimistic first-chat echo",
      ),
    );

    rerender({ selected: OTHER_ACTOR_ID });
    await waitFor(() =>
      expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID),
    );
    expect(
      result.current.session?.messages.some(
        (message) => message.content === "Optimistic first-chat echo",
      ),
    ).toBe(false);

    await act(async () => {
      stream.push({ runStarted: { actorId: ACTOR_ID, runId: "turn-echo" } });
      stream.push({ runFinished: { runId: "turn-echo", result: {} } });
      stream.close();
      await sendPromise!;
    });
    expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID);
  });

  it("a queued canonical swap cannot override a newer conversation choice", async () => {
    const stream = controlledSse();
    const adopted = vi.fn();
    installSwitchingMock(stream);
    const { result, rerender } = renderHook(
      ({ selected }: { selected?: string }) =>
        useAssistantChat({
          selectedConversationId: selected,
          onConversationAdopted: adopted,
        }),
      { initialProps: { selected: undefined as string | undefined } },
    );
    await waitFor(() => expect(result.current.listLoading).toBe(false));
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Create a canonical chat");
    });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    rerender({ selected: OTHER_ACTOR_ID });
    await waitFor(() =>
      expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID),
    );
    await act(async () => {
      stream.push({
        runStarted: { actorId: ACTOR_ID, runId: "turn-adopt-late" },
      });
      stream.push({ runFinished: { runId: "turn-adopt-late", result: {} } });
      stream.close();
      await sendPromise!;
    });

    expect(adopted).not.toHaveBeenCalled();
    expect(result.current.session?.conversationId).toBe(OTHER_ACTOR_ID);
    expect(
      result.current.visibleConversations.map((item) => item.id),
    ).toContain(ACTOR_ID);
  });

  it("dispatches every typed control with the fresh version and refreshes after 202", async () => {
    const commands: Record<string, unknown>[] = [];
    let stateReads = 0;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [META] });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
        return json(storedDetail(ACTOR_ID, "Saved chat"));
      }
      if (endpoint.startsWith(`/assistant/conversations/${ACTOR_ID}/state`)) {
        stateReads += 1;
        return json(actorControlState());
      }
      if (endpoint === "/assistant/chat") {
        const body = JSON.parse(String(init.body)) as Record<string, unknown>;
        commands.push(body);
        return body.type === "action.continue"
          ? sse([
              { runStarted: { actorId: ACTOR_ID, runId: "turn-action" } },
              { runFinished: { runId: "turn-action", result: {} } },
            ])
          : json({ accepted: true }, 202);
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() => expect(result.current.controlReady).toBe(true));
    const projection = result.current.projection!;
    const input = projection.pendingInput!;
    const approval = projection.pendingApproval!;
    const gate = projection.task!.gate!;
    const step = projection.steps.get("step-active")!;

    await act(async () =>
      result.current.resolveInput({ selectedOptionIds: ["option-sg"] }, input),
    );
    await act(async () =>
      result.current.resolveApproval(
        approval.approvalRequestId,
        false,
        "Not now",
      ),
    );
    await act(async () => result.current.resolvePlan(true, gate));
    await act(async () => result.current.steer("Keep the current scope"));
    await act(async () => result.current.controlStep("step.retry", step));
    await act(async () => result.current.controlStep("step.skip", step));
    await act(async () => result.current.stop());
    await act(async () =>
      result.current.reportAction({
        actionRequestId: "action-active",
        originTurnId: "turn-active",
        disposition: "declined",
      }),
    );

    expect(commands.map((command) => command.type)).toEqual([
      "input.resolve",
      "approval.resolve",
      "plan.resolve",
      "task.steer",
      "step.retry",
      "step.skip",
      "task.stop",
      "action.continue",
    ]);
    for (const command of commands.slice(0, 7)) {
      expect(command.expectedStateVersion).toBe(7);
    }
    expect(commands[1]).toMatchObject({ approved: false, reason: "Not now" });
    expect(commands[4]).toMatchObject({ expectedOperationGeneration: 4 });
    expect(commands[5]).toMatchObject({ expectedOperationGeneration: 4 });
    expect(stateReads).toBeGreaterThanOrEqual(10);
    expect(
      result.current.session?.messages.some(
        (message) => message.content === "NyxID action update: declined.",
      ),
    ).toBe(true);
  });

  it("fences all typed controls until a positive current state is available", async () => {
    const commands: Record<string, unknown>[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [META] });
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}`) {
        return json(storedDetail(ACTOR_ID, "Saved chat"));
      }
      if (endpoint === `/assistant/conversations/${ACTOR_ID}/state`) {
        return json(actorControlState(0));
      }
      if (endpoint === "/assistant/chat") {
        commands.push(JSON.parse(String(init.body)) as Record<string, unknown>);
        return json({ accepted: true }, 202);
      }
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() =>
      expect(result.current.projection?.task?.taskId).toBe("task-active"),
    );
    expect(result.current.controlReady).toBe(false);
    const projection = result.current.projection!;

    await act(async () =>
      result.current.resolveInput(
        { selectedOptionIds: ["option-sg"] },
        projection.pendingInput!,
      ),
    );
    await act(async () =>
      result.current.resolveApproval(
        projection.pendingApproval!.approvalRequestId,
        true,
      ),
    );
    await act(async () =>
      result.current.resolvePlan(true, projection.task!.gate!),
    );
    await act(async () => result.current.steer("Do not send"));
    await act(async () =>
      result.current.controlStep(
        "step.retry",
        projection.steps.get("step-active")!,
      ),
    );
    await act(async () => result.current.stop());

    expect(commands).toEqual([]);
  });

  it("deletes a settled conversation and opens a fresh draft", async () => {
    const deletes: string[] = [];
    const original = globalThis.__nyxidAssistantHttpMock;
    globalThis.__nyxidAssistantHttpMock = (request) => {
      if (
        request.endpoint === `/assistant/conversations/${ACTOR_ID}` &&
        request.init.method === "DELETE"
      ) {
        deletes.push(request.endpoint);
        return new Response(null, { status: 204 });
      }
      return original?.(request);
    };
    const { result, rerender } = renderHook(
      ({ conversationId }: { conversationId: string | undefined }) =>
        useAssistantChat({ selectedConversationId: conversationId }),
      { initialProps: { conversationId: ACTOR_ID as string | undefined } },
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );

    await act(async () => result.current.deleteConversation(ACTOR_ID));

    expect(deletes).toEqual([`/assistant/conversations/${ACTOR_ID}`]);
    expect(result.current.visibleConversations).toEqual([]);
    expect(result.current.session).toMatchObject({
      status: "draft",
      title: "New chat",
    });
    const draftClientId = result.current.session?.clientId;
    rerender({ conversationId: undefined });
    await waitFor(() =>
      expect(result.current.session?.clientId).toBe(draftClientId),
    );
  });

  it("prunes superseded empty drafts when New chat is repeated", async () => {
    const { result, rerender } = renderHook(
      ({ conversationId }: { conversationId: string | undefined }) =>
        useAssistantChat({ selectedConversationId: conversationId }),
      { initialProps: { conversationId: undefined } },
    );
    await waitFor(() => expect(result.current.listLoading).toBe(false));
    const initialClientId = result.current.session?.clientId;
    let latestClientId: string | undefined;
    act(() => {
      result.current.newChat();
      latestClientId = result.current.newChat();
    });

    expect(latestClientId).not.toBe(initialClientId);
    expect(result.current.session?.clientId).toBe(latestClientId);
    expect(result.current.visibleConversations).toEqual([META]);
    rerender({ conversationId: undefined });
    expect(result.current.session?.clientId).toBe(latestClientId);
  });

  it("refuses deletion while that conversation is streaming", async () => {
    const stream = controlledSse();
    const deletes: string[] = [];
    installSwitchingMock(stream);
    const original = globalThis.__nyxidAssistantHttpMock;
    globalThis.__nyxidAssistantHttpMock = (request) => {
      if (request.init.method === "DELETE") {
        deletes.push(request.endpoint);
        return new Response(null, { status: 204 });
      }
      return original?.(request);
    };
    const { result } = renderHook(() =>
      useAssistantChat({ selectedConversationId: ACTOR_ID }),
    );
    await waitFor(() =>
      expect(result.current.session?.messages).toHaveLength(2),
    );
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Still running");
    });
    await act(async () => {
      stream.push({ runStarted: { actorId: ACTOR_ID, runId: "turn-delete" } });
    });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    await act(async () => result.current.deleteConversation(ACTOR_ID));
    expect(deletes).toEqual([]);

    await act(async () => {
      stream.push({
        runStopped: { runId: "turn-delete", reason: "test_done" },
      });
      stream.close();
      await sendPromise!;
    });
  });

  it("bounds active-state refreshes while a version-zero stream remains open", async () => {
    vi.useFakeTimers();
    const stream = controlledSse();
    let stateReads = 0;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint === "/assistant/conversations") {
        return json({ conversations: [] });
      }
      if (endpoint.startsWith(`/assistant/conversations/${ACTOR_ID}/state`)) {
        stateReads += 1;
        return json({ status: "not_found" });
      }
      if (endpoint === "/assistant/chat") return stream.response;
      throw new Error(`Unhandled assistant mock request: ${endpoint}`);
    };
    const { result } = renderHook(() => useAssistantChat({}));
    await act(async () => vi.advanceTimersByTimeAsync(0));
    let sendPromise: Promise<void>;
    act(() => {
      sendPromise = result.current.send("Wait for actor state");
    });
    await act(async () => {
      stream.push({ runStarted: { actorId: ACTOR_ID, runId: "turn-refresh" } });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(stateReads).toBe(1);

    await act(async () => vi.advanceTimersByTimeAsync(3_750));
    expect(stateReads).toBe(5);
    expect(result.current.controlReady).toBe(false);
    expect(result.current.session?.status).toBe("streaming");

    await act(async () => {
      stream.push({
        runStopped: { runId: "turn-refresh", reason: "test_done" },
      });
      stream.close();
      await sendPromise!;
    });
  });
});
