import { beforeEach, describe, expect, it, vi } from "vitest";
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

    transport.sendMessage(conversationId, "go", () => undefined);
    await vi.waitFor(() => expect(useAuthStore.getState().user).toBeNull());
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
    });
    expect(useAssistantDraftStore.getState().drafts).toEqual({});
    await expect(transport.getHistory(conversationId)).rejects.toThrow(
      "Conversation was not found",
    );
  });
});
