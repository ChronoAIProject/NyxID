import { beforeEach, describe, expect, it, vi } from "vitest";
import errorThenDeltaFixture from "@/lib/assistant/__fixtures__/chrono-llm-direct-error-then-delta.sse?raw";
import fixture from "@/lib/assistant/__fixtures__/chrono-llm-direct-stream.sse?raw";
import { DirectAssistantTransport } from "@/lib/assistant/direct-transport";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAuthStore } from "@/stores/auth-store";
import type { TurnEvent } from "@/types/assistant";
import type { User } from "@/types/api";

const encoder = new TextEncoder();

function user(id: string): User {
  return {
    id,
    email: `${id}@example.com`,
    display_name: id,
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-08-11T00:00:00.000Z",
  };
}

function sseResponse(body: string, status = 200): Response {
  return new Response(body, {
    status,
    headers: { "Content-Type": "text/event-stream" },
  });
}

function chunkedSseResponse(body: string, onCancel?: () => void): Response {
  const chunks = body
    .trim()
    .split("\n\n")
    .map((chunk) => encoder.encode(`${chunk}\n\n`));
  let nextChunk = 0;
  return new Response(
    new ReadableStream<Uint8Array>({
      pull(controller) {
        const chunk = chunks[nextChunk];
        if (!chunk) {
          controller.close();
          return;
        }
        nextChunk += 1;
        controller.enqueue(chunk);
        if (nextChunk === chunks.length) controller.close();
      },
      cancel() {
        onCancel?.();
      },
    }),
    { status: 200, headers: { "Content-Type": "text/event-stream" } },
  );
}

async function waitForTerminal(events: readonly TurnEvent[]): Promise<void> {
  await vi.waitFor(() => {
    expect(
      events.filter((event) => event.event === "turn.completed"),
    ).toHaveLength(1);
  });
}

async function startedConversation(
  transport: DirectAssistantTransport,
): Promise<string> {
  return (await transport.createConversation()).id;
}

beforeEach(() => {
  localStorage.clear();
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
  transitionAssistantIdentity(null);
  useAuthStore.getState().setUser(user("user-a"));
});

describe("DirectAssistantTransport streaming", () => {
  it("seeds default models silently: no listener call, no-op identity kept", async () => {
    const transport = new DirectAssistantTransport();
    const listener = vi.fn();
    const unsubscribe = transport.subscribeSettings(listener);

    const draftBefore = transport.getSettings();
    transport.seedDefaultModel(undefined, draftBefore.model);
    expect(transport.getSettings()).toBe(draftBefore);
    expect(listener).not.toHaveBeenCalled();

    const conversationId = await startedConversation(transport);
    const settingsBefore = transport.getSettings(conversationId);
    const conversationBefore = (await transport.listConversations())[0];
    transport.seedDefaultModel(conversationId, settingsBefore.model);
    expect(transport.getSettings(conversationId)).toBe(settingsBefore);
    expect((await transport.listConversations())[0]).toBe(conversationBefore);
    expect(listener).not.toHaveBeenCalled();

    // Real seeds run during render, so they update the snapshot without
    // notifying subscribers (the caller reads the store after seeding).
    transport.seedDefaultModel(conversationId, "server-new");
    expect(listener).not.toHaveBeenCalled();
    expect(transport.getSettings(conversationId).model).toBe("server-new");
    expect((await transport.listConversations())[0]?.llm_model).toBe(
      "server-new",
    );
    unsubscribe();
  });

  it("accumulates the saved fixture and sends the selected model and skill", async () => {
    const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
      void args;
      return sseResponse(fixture);
    });
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await startedConversation(transport);
    transport.setModel(conversationId, "gpt-5.4-mini");
    transport.setSkill(conversationId, "nyxid");
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "Hello", (event) =>
      events.push(event),
    );
    await waitForTerminal(events);

    const request = fetchMock.mock.calls[0]?.[1];
    expect(JSON.parse(String(request?.body))).toEqual({
      messages: [{ role: "user", content: "Hello" }],
      model: "gpt-5.4-mini",
      skill_slug: "nyxid",
    });
    const history = await transport.getHistory(conversationId);
    expect(history.messages.at(-1)?.blocks).toEqual([
      expect.objectContaining({
        type: "text",
        text: "Hello, friend, welcome here SKILLMARK",
      }),
    ]);
    expect(history.conversation.llm_model).toBe("gpt-5.4-mini");
    expect(history.messages.at(-1)?.status).toBeUndefined();
    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "completed",
    });
  });

  it("sends the selected effort and omits it entirely when unset", async () => {
    const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
      void args;
      return sseResponse(fixture);
    });
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "no effort", (event) =>
      events.push(event),
    );
    await waitForTerminal(events);
    const withoutEffort = JSON.parse(
      String(fetchMock.mock.calls[0]?.[1]?.body),
    ) as Record<string, unknown>;
    expect(withoutEffort).not.toHaveProperty("effort");

    transport.setEffort(conversationId, "xhigh");
    const second: TurnEvent[] = [];
    transport.sendMessage(conversationId, "with effort", (event) =>
      second.push(event),
    );
    await waitForTerminal(second);
    expect(
      JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body)),
    ).toMatchObject({ effort: "xhigh" });

    transport.setEffort(conversationId, null);
    const third: TurnEvent[] = [];
    transport.sendMessage(conversationId, "cleared", (event) =>
      third.push(event),
    );
    await waitForTerminal(third);
    expect(
      JSON.parse(String(fetchMock.mock.calls[2]?.[1]?.body)),
    ).not.toHaveProperty("effort");
  });

  it("keeps later replies when the upstream reuses a completion id", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(fixture)),
    });
    const conversationId = await startedConversation(transport);

    for (const prompt of ["first", "second"]) {
      const events: TurnEvent[] = [];
      transport.sendMessage(conversationId, prompt, (event) =>
        events.push(event),
      );
      await waitForTerminal(events);
    }

    const history = await transport.getHistory(conversationId);
    expect(history.messages).toHaveLength(4);
    expect(
      history.messages
        .filter((message) => message.role === "assistant")
        .map((message) =>
          message.blocks
            .filter((block) => block.type === "text")
            .map((block) => (block.type === "text" ? block.text : ""))
            .join(""),
        ),
    ).toEqual([
      "Hello, friend, welcome here SKILLMARK",
      "Hello, friend, welcome here SKILLMARK",
    ]);
    const assistantIds = history.messages
      .filter((message) => message.role === "assistant")
      .map((message) => message.id);
    expect(new Set(assistantIds).size).toBe(2);
  });

  it("calls the global fetch without rebinding this (default fetch path)", async () => {
    // Regression: the transport defaulted `fetchFn` to the bare global
    // `fetch`, then invoked it as `this.fetchFn(...)`. Real browsers throw
    // "Illegal invocation" because `fetch` must run bound to `window`; jsdom
    // does not enforce this, so the stub below reproduces the guard itself,
    // rejecting any call whose `this` is not the global. Every other test
    // injects a fetch mock, so only the default path exercises this.
    const realFetch = globalThis.fetch;
    const guardedFetch = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(function (this: unknown, ...args) {
        if (this !== undefined && this !== globalThis) {
          throw new TypeError("Illegal invocation");
        }
        void args;
        return Promise.resolve(sseResponse(fixture));
      });
    try {
      const transport = new DirectAssistantTransport();
      const conversationId = await startedConversation(transport);
      const events: TurnEvent[] = [];
      transport.sendMessage(conversationId, "Hello", (event) =>
        events.push(event),
      );
      await waitForTerminal(events);
      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "completed",
      });
      expect(guardedFetch).toHaveBeenCalledTimes(1);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  it("keeps a 70-message transcript inside the server request caps", async () => {
    let responseIndex = 0;
    const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
      void args;
      responseIndex += 1;
      return sseResponse(
        `data: {"id":"assistant-${String(responseIndex)}","choices":[{"delta":{"content":"reply-${String(responseIndex)}"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n`,
      );
    });
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await startedConversation(transport);

    for (let index = 0; index < 35; index += 1) {
      const events: TurnEvent[] = [];
      transport.sendMessage(
        conversationId,
        `${String(index).padStart(2, "0")}:${"x".repeat(9_997)}`,
        (event) => events.push(event),
      );
      await waitForTerminal(events);
    }
    expect((await transport.getHistory(conversationId)).messages).toHaveLength(
      70,
    );

    fetchMock.mockClear();
    const events: TurnEvent[] = [];
    transport.sendMessage(conversationId, "newest user turn", (event) =>
      events.push(event),
    );
    await waitForTerminal(events);

    const requestBody = String(fetchMock.mock.calls[0]?.[1]?.body);
    const request = JSON.parse(requestBody) as {
      messages: Array<{ role: string; content: string }>;
    };
    const aggregateBytes = encoder.encode(
      request.messages.map((message) => message.content).join(""),
    ).byteLength;
    expect(request.messages.length).toBeLessThanOrEqual(63);
    expect(aggregateBytes).toBeLessThanOrEqual(256 * 1024);
    expect(encoder.encode(requestBody).byteLength).toBeLessThanOrEqual(
      256 * 1024,
    );
    expect(request.messages.at(-1)).toEqual({
      role: "user",
      content: "newest user turn",
    });
  });

  it("keeps reading after finish_reason through usage and [DONE]", async () => {
    const frames = [
      'data: {"id":"m","choices":[{"delta":{"content":"answer"},"finish_reason":"stop"}]}\n\n',
      'data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}\n\n',
      "data: [DONE]\n\n",
    ];
    let index = 0;
    let usageConsumed = false;
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        const frame = frames[index++];
        if (frame === undefined) {
          controller.close();
          return;
        }
        if (frame.includes('"usage"')) usageConsumed = true;
        controller.enqueue(encoder.encode(frame));
      },
    });
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => new Response(stream, { status: 200 })),
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);
    await vi.waitFor(() => expect(usageConsumed).toBe(true));
    await vi.waitFor(async () =>
      expect(await transport.listConversations()).toHaveLength(1),
    );
  });

  it.each([
    {
      name: "finish_reason followed by EOF",
      body: 'data: {"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}\n\n',
      status: "completed",
    },
    {
      name: "[DONE] followed by EOF",
      body: 'data: {"choices":[{"delta":{"content":"ok"},"finish_reason":null}]}\n\ndata: [DONE]\n\n',
      status: "completed",
    },
    {
      name: "bare EOF",
      body: 'data: {"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}\n\n',
      status: "failed",
    },
  ])("settles $name as $status", async ({ body, status }) => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(body)),
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status,
    });
  });

  it("ignores payloads after an upstream error frame", async () => {
    const streamCancelled = vi.fn();
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () =>
        chunkedSseResponse(errorThenDeltaFixture, streamCancelled),
      ),
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(events.filter((event) => event.event === "turn.completed")).toEqual([
      expect.objectContaining({
        status: "failed",
        error: { code: "upstream_failed", message: "Upstream failed" },
      }),
    ]);
    expect(events.filter((event) => event.event === "message.started")).toEqual(
      [],
    );
    expect(events.findIndex((event) => event.event === "turn.completed")).toBe(
      events.length - 1,
    );
    expect(streamCancelled).toHaveBeenCalledOnce();
  });

  it("aborts and emits cancellation exactly once", async () => {
    const fetchMock = vi.fn(
      async (_url: RequestInfo | URL, init?: RequestInit) => {
        const stream = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(
              encoder.encode(
                'data: {"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}\n\n',
              ),
            );
            init?.signal?.addEventListener("abort", () =>
              controller.error(new DOMException("Aborted", "AbortError")),
            );
          },
        });
        return new Response(stream, { status: 200 });
      },
    );
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];
    const handle = transport.sendMessage(conversationId, "go", (event) =>
      events.push(event),
    );
    await vi.waitFor(() =>
      expect(events.some((event) => event.event === "block.delta")).toBe(true),
    );

    handle.cancel();
    handle.cancel();
    await waitForTerminal(events);

    expect(events.filter((event) => event.event === "turn.completed")).toEqual([
      expect.objectContaining({ status: "cancelled" }),
    ]);
  });

  it("fails a request that misses the first-byte deadline", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(() => new Promise<Response>(() => undefined)),
      firstByteTimeoutMs: 5,
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "first_byte_timeout" },
    });
  });

  it("fails a stream that exceeds the idle deadline", async () => {
    const fetchMock = vi.fn(
      async (_url: RequestInfo | URL, init?: RequestInit) => {
        const stream = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(
              encoder.encode(
                'data: {"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}\n\n',
              ),
            );
            init?.signal?.addEventListener("abort", () =>
              controller.error(new DOMException("Aborted", "AbortError")),
            );
          },
        });
        return new Response(stream, { status: 200 });
      },
    );
    const transport = new DirectAssistantTransport({
      fetch: fetchMock,
      idleTimeoutMs: 5,
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "idle_timeout" },
    });
  });
});

describe("DirectAssistantTransport authentication boundaries", () => {
  it("clears auth only for a structured NyxID 401", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              error: "unauthorized",
              error_code: 1001,
              message: "Session expired",
            }),
            { status: 401, headers: { "Content-Type": "application/json" } },
          ),
      ),
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);
    await vi.waitFor(() => expect(useAuthStore.getState().user).toBeNull());
    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "http_401", message: "Session expired" },
    });
  });

  it("surfaces a structured NyxID 4xx message for residual cap failures", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              error: "bad_request",
              error_code: 1000,
              message: "Conversation too long. Start a new chat.",
            }),
            { status: 400, headers: { "Content-Type": "application/json" } },
          ),
      ),
    });
    const conversationId = await startedConversation(transport);
    const events: TurnEvent[] = [];

    transport.sendMessage(conversationId, "go", (event) => events.push(event));
    await waitForTerminal(events);

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: {
        code: "http_400",
        message: "Conversation too long. Start a new chat.",
      },
    });
  });

  it.each([
    {
      status: 401,
      body: { error: { message: "upstream credential rejected" } },
    },
    {
      status: 403,
      body: { error: { message: "upstream credential forbidden" } },
    },
    {
      status: 404,
      body: {
        error: "not_found",
        error_code: 1004,
        message: "Assistant route not found.",
      },
    },
  ])(
    "preserves the NyxID session for downstream/flag-off $status",
    async ({ status, body }) => {
      const transport = new DirectAssistantTransport({
        fetch: vi.fn(
          async () =>
            new Response(JSON.stringify(body), {
              status,
              headers: { "Content-Type": "application/json" },
            }),
        ),
      });
      const conversationId = await startedConversation(transport);
      const events: TurnEvent[] = [];

      transport.sendMessage(conversationId, "go", (event) =>
        events.push(event),
      );
      await waitForTerminal(events);

      expect(useAuthStore.getState().user?.id).toBe("user-a");
      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
      });
    },
  );
});

describe("DirectAssistantTransport identity isolation", () => {
  it("wipes transcript, running fetch, picker state, and drafts across A -> logout -> B", async () => {
    let aborted = false;
    const fetchMock = vi.fn(
      async (_url: RequestInfo | URL, init?: RequestInit) => {
        const stream = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(
              encoder.encode(
                'data: {"choices":[{"delta":{"content":"private"},"finish_reason":null}]}\n\n',
              ),
            );
            init?.signal?.addEventListener("abort", () => {
              aborted = true;
              controller.error(new DOMException("Aborted", "AbortError"));
            });
          },
        });
        return new Response(stream, { status: 200 });
      },
    );
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await startedConversation(transport);
    transport.setModel(conversationId, "gpt-5.2");
    transport.setSkill(conversationId, "nyxid");
    useAssistantDraftStore
      .getState()
      .saveDraft("user-a", `conv:${conversationId}`, "private draft");
    transport.sendMessage(conversationId, "private prompt", () => undefined);
    await vi.waitFor(async () => {
      const history = await transport.getHistory(conversationId);
      expect(history.messages).toHaveLength(2);
    });

    useAuthStore.getState().setUser(null);
    useAuthStore.getState().setUser(user("user-b"));

    await vi.waitFor(() => expect(aborted).toBe(true));
    expect(transport.getOwnerUserId()).toBe("user-b");
    expect(await transport.listConversations()).toEqual([]);
    expect(transport.getSettings()).toEqual({
      model: "gpt-5.5",
      skillSlug: null,
      effort: null,
      agentPocMode: false,
    });
    expect(useAssistantDraftStore.getState().drafts).toEqual({});
    await expect(transport.getHistory(conversationId)).rejects.toThrow(
      "Conversation was not found",
    );
  });
});
