import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import deadlineFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-deadline.sse?raw";
import doneMissingFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-done-missing.sse?raw";
import duplicateTerminalFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-duplicate-terminal.sse?raw";
import heartbeatFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-heartbeat.sse?raw";
import malformedStageFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-malformed-stage.sse?raw";
import happyFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-run.sse?raw";
import toolErrorFixture from "@/lib/assistant/__fixtures__/chrono-llm-agent-tool-error.sse?raw";
import directFixture from "@/lib/assistant/__fixtures__/chrono-llm-direct-stream.sse?raw";
import {
  DIRECT_AGENT_POC_URL,
  MAX_AGENT_POC_CONTENT_BYTES,
  projectAgentPocMessages,
} from "@/lib/assistant/direct-agent-poc";
import { isAgentPocPlanBlockId as leafPredicate } from "@/lib/assistant/direct-agent-poc-ids";
import {
  DirectAssistantTransport,
  isAgentPocPlanBlockId,
} from "@/lib/assistant/direct-transport";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { RunCard } from "@/components/assistant/blocks/run-card";
import { useAuthStore } from "@/stores/auth-store";
import type {
  AssistantMessage,
  RunContentBlock,
  TurnEvent,
} from "@/types/assistant";
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
    created_at: "2026-08-13T00:00:00.000Z",
  };
}

function sseResponse(body: string): Response {
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  });
}

function hangingResponse(prefix: string, signal?: AbortSignal): Response {
  return new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode(prefix));
        signal?.addEventListener("abort", () =>
          controller.error(new DOMException("Aborted", "AbortError")),
        );
      },
    }),
    { status: 200, headers: { "Content-Type": "text/event-stream" } },
  );
}

async function createPocConversation(
  transport: DirectAssistantTransport,
): Promise<string> {
  const conversationId = (await transport.createConversation()).id;
  transport.setAgentPocMode(conversationId, true);
  return conversationId;
}

async function send(
  transport: DirectAssistantTransport,
  conversationId: string,
  content = "check health",
): Promise<TurnEvent[]> {
  const events: TurnEvent[] = [];
  transport.sendMessage(conversationId, content, (event) => events.push(event));
  await vi.waitFor(() =>
    expect(
      events.filter((event) => event.event === "turn.completed"),
    ).toHaveLength(1),
  );
  return events;
}

function terminal(events: readonly TurnEvent[]) {
  return [...events]
    .reverse()
    .find((event) => event.event === "turn.completed");
}

function lastAssistant(
  messages: readonly AssistantMessage[],
): AssistantMessage | undefined {
  return [...messages]
    .reverse()
    .find((message) => message.role === "assistant");
}

function runBlock(messages: readonly AssistantMessage[]): RunContentBlock {
  const block = lastAssistant(messages)?.blocks.find(
    (candidate) => candidate.type === "run",
  );
  expect(block?.type).toBe("run");
  return block as RunContentBlock;
}

function assertSettled(run: RunContentBlock): void {
  expect(
    run.steps.filter(
      (step) => step.status === "active" || step.status === "waiting",
    ),
  ).toEqual([]);
}

beforeEach(() => {
  localStorage.clear();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
  transitionAssistantIdentity(null);
  useAuthStore.getState().setUser(user("agent-poc-user"));
});

describe("Direct agent POC fixture contract", () => {
  it("renders one completed assistant message with run, plan, and final blocks", async () => {
    const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
      void args;
      return sseResponse(happyFixture);
    });
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);
    const history = await transport.getHistory(conversationId);
    const assistant = lastAssistant(history.messages);
    const run = runBlock(history.messages);

    expect(fetchMock.mock.calls[0]?.[0]).toBe(DIRECT_AGENT_POC_URL);
    expect(terminal(events)).toMatchObject({
      status: "completed",
      error: null,
    });
    expect(assistant?.blocks).toHaveLength(3);
    expect(run.state).toBe("completed");
    expect(run.steps.map((step) => step.label)).toEqual([
      "Understand",
      "Plan",
      "Execute",
      "chrono-sandbox · health_handler",
      "Report",
    ]);
    expect(run.steps.map((step) => step.status)).toEqual([
      "done",
      "done",
      "done",
      "done",
      "done",
    ]);
    expect(
      run.steps.find((step) => step.label.includes("health_handler"))
        ?.result_preview,
    ).toBe('{"healthy":true,"request_id":"health-42"}');
    expect(assistant?.blocks[1]).toMatchObject({
      type: "text",
      text: "1. Check health.",
    });
    expect(isAgentPocPlanBlockId(assistant?.blocks[1]?.block_id ?? "")).toBe(
      true,
    );
    expect(assistant?.blocks[2]).toMatchObject({
      type: "text",
      text: "Chrono Sandbox is healthy.",
    });
    render(createElement(RunCard, { block: run }));
    fireEvent.click(screen.getByText("Result preview"));
    expect(screen.getByText(/health-42/)).toBeVisible();
    assertSettled(run);
  });

  it("keeps a failed tool step while allowing a grounded successful run", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(toolErrorFixture)),
    });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);
    const run = runBlock((await transport.getHistory(conversationId)).messages);

    expect(terminal(events)).toMatchObject({ status: "completed" });
    expect(run.state).toBe("completed");
    expect(
      run.steps.find((step) => step.meta.includes("HTTP 503"))?.status,
    ).toBe("failed");
    assertSettled(run);
  });

  it("accepts an older tool completion without a result preview", async () => {
    const legacyFixture = happyFixture
      .split("\n")
      .map((line) => {
        if (!line.startsWith("data: {")) return line;
        const frame = JSON.parse(line.slice("data: ".length)) as Record<
          string,
          unknown
        >;
        if (frame.type !== "tool.completed") return line;
        delete frame.result_preview;
        return `data: ${JSON.stringify(frame)}`;
      })
      .join("\n");
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(legacyFixture)),
    });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);
    const run = runBlock((await transport.getHistory(conversationId)).messages);

    expect(terminal(events)).toMatchObject({ status: "completed", error: null });
    expect(
      run.steps.find((step) => step.label.includes("health_handler"))
        ?.result_preview,
    ).toBeNull();
    assertSettled(run);
  });

  it.each([
    ["deadline", deadlineFixture, "deadline_exceeded"],
    ["malformed known stage", malformedStageFixture, "protocol_error"],
    ["duplicate terminal", duplicateTerminalFixture, "protocol_error"],
    ["done without terminal", doneMissingFixture, "truncated_stream"],
  ])("settles %s universally", async (_name, fixture, code) => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(fixture)),
    });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);
    const run = runBlock((await transport.getHistory(conversationId)).messages);

    expect(terminal(events)).toMatchObject({
      status: "failed",
      error: { code },
    });
    expect(run.state).toBe("failed");
    if (_name === "duplicate terminal") {
      expect(run.steps.map((step) => step.label)).toEqual([
        "Understand",
        "Plan",
        "Execute",
        "Report",
      ]);
      expect(run.steps.map((step) => step.status)).toEqual([
        "done",
        "done",
        "done",
        "done",
      ]);
    }
    assertSettled(run);
  });

  it("accepts heartbeat comments and a terminal response with no final text", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(heartbeatFixture)),
    });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);
    const history = await transport.getHistory(conversationId);
    const assistant = lastAssistant(history.messages);

    expect(terminal(events)).toMatchObject({ status: "completed" });
    expect(runBlock(history.messages).state).toBe("completed");
    expect(assistant?.blocks.filter((block) => block.type === "text")).toEqual(
      [],
    );
  });

  it("ignores unknown types, including inherited Object prototype keys", async () => {
    const inheritedKeyFrame =
      'data: {"type":"toString","secret":"ignored"}\n\n';
    const fixture = happyFixture.replace(
      'data: {"type":"stage","stage":"understand"',
      `${inheritedKeyFrame}data: {"type":"future.frame","value":1}\n\ndata: {"type":"stage","stage":"understand"`,
    );
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(fixture)),
    });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);

    expect(terminal(events)).toMatchObject({
      status: "completed",
      error: null,
    });
  });

  it.each([
    [
      "an out-of-order stage",
      [
        'data: {"type":"run.started","run_id":"bad-order","model":"gpt-5.5","skill_slug":null,"effort":null,"limits":{"max_tool_calls":8,"max_llm_calls":8,"deadline_ms":180000}}',
        'data: {"type":"stage","stage":"plan","status":"started","detail":"too early"}',
      ].join("\n\n"),
    ],
    [
      "a completed terminal with an active stage",
      [
        'data: {"type":"run.started","run_id":"active-stage","model":"gpt-5.5","skill_slug":null,"effort":null,"limits":{"max_tool_calls":8,"max_llm_calls":8,"deadline_ms":180000}}',
        'data: {"type":"stage","stage":"understand","status":"started","detail":"still active"}',
        'data: {"type":"done","status":"completed","finish_reason":"stop","tool_calls":0,"llm_calls":1,"duration_ms":10}',
        "data: [DONE]",
      ].join("\n\n"),
    ],
    [
      "a tool completion with a different identity",
      happyFixture.replace(
        '"call_id":"call-1","tool":"nyx_call_tool","outcome":"ok"',
        '"call_id":"call-1","tool":"nyx_get_skill","outcome":"ok"',
      ),
    ],
    [
      "plan text outside the active plan stage",
      happyFixture.replace(
        'data: {"type":"stage","stage":"plan","status":"started","detail":"Building a bounded read-only plan"}\n\n',
        "",
      ),
    ],
  ])("rejects %s as a protocol error", async (_name, fixture) => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () => sseResponse(`${fixture}\n\n`)),
    });
    const conversationId = await createPocConversation(transport);

    const events = await send(transport, conversationId);
    const run = runBlock((await transport.getHistory(conversationId)).messages);

    expect(terminal(events)).toMatchObject({
      status: "failed",
      error: { code: "protocol_error" },
    });
    expect(run.state).toBe("failed");
    assertSettled(run);
  });

  it("maps fetch failures and first-byte and idle deadlines to stable errors", async () => {
    const cases: Array<{
      expected: string;
      transport: DirectAssistantTransport;
    }> = [
      {
        expected: "network_error",
        transport: new DirectAssistantTransport({
          fetch: vi.fn(async () => {
            throw new TypeError("offline");
          }),
        }),
      },
      {
        expected: "first_byte_timeout",
        transport: new DirectAssistantTransport({
          fetch: vi.fn(() => new Promise<Response>(() => undefined)),
          firstByteTimeoutMs: 5,
        }),
      },
      {
        expected: "idle_timeout",
        transport: new DirectAssistantTransport({
          fetch: vi.fn(async (_url, init) =>
            hangingResponse(
              'data: {"type":"run.started","run_id":"idle","model":"gpt-5.5","skill_slug":null,"effort":null,"limits":{"max_tool_calls":8,"max_llm_calls":8,"deadline_ms":180000}}\n\n',
              init?.signal ?? undefined,
            ),
          ),
          idleTimeoutMs: 5,
        }),
      },
    ];

    for (const { expected, transport } of cases) {
      const conversationId = await createPocConversation(transport);
      const events = await send(transport, conversationId);
      expect(terminal(events)).toMatchObject({
        status: "failed",
        error: { code: expected },
      });
      const history = await transport.getHistory(conversationId);
      const assistant = lastAssistant(history.messages);
      if (assistant) assertSettled(runBlock(history.messages));
    }
  });

  it("cancels mid-execute and settles every active step", async () => {
    const prefix = happyFixture.split('data: {"type":"tool.completed"')[0]!;
    const fetchMock = vi.fn(
      async (_url: RequestInfo | URL, init?: RequestInit) =>
        hangingResponse(prefix, init?.signal ?? undefined),
    );
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await createPocConversation(transport);
    const events: TurnEvent[] = [];
    const handle = transport.sendMessage(conversationId, "cancel", (event) =>
      events.push(event),
    );
    await vi.waitFor(() =>
      expect(
        events.some(
          (event) =>
            event.event === "block.updated" &&
            event.patch.type === "run" &&
            event.patch.steps?.some(
              (step) => step.label === "chrono-sandbox · health_handler",
            ) === true,
        ),
      ).toBe(true),
    );

    handle.cancel();
    await vi.waitFor(() =>
      expect(terminal(events)).toMatchObject({ status: "cancelled" }),
    );
    const run = runBlock((await transport.getHistory(conversationId)).messages);
    expect(run.state).toBe("cancelled");
    assertSettled(run);
  });
});

describe("Direct agent POC transport and transcript boundaries", () => {
  it("has a neutral predicate leaf and no evaluation-time import cycle", () => {
    expect(isAgentPocPlanBlockId).toBe(leafPredicate);
    expect(leafPredicate("message-agent-poc-plan")).toBe(true);
    expect(projectAgentPocMessages([])).toEqual([]);
  });

  it("delivers a structured NyxID 401 terminal before invalidating identity", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () =>
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
    const conversationId = await createPocConversation(transport);
    const events: TurnEvent[] = [];
    let identityAtTerminal: string | null | undefined;

    transport.sendMessage(conversationId, "expire", (event) => {
      events.push(event);
      if (event.event === "turn.completed") {
        identityAtTerminal = useAuthStore.getState().user?.id;
      }
    });
    await vi.waitFor(() => expect(terminal(events)).toBeDefined());
    expect(identityAtTerminal).toBe("agent-poc-user");
    expect(terminal(events)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "http_401", message: "Session expired" },
    });
    await vi.waitFor(() => expect(useAuthStore.getState().user).toBeNull());
  });

  it("preserves identity for an unstructured POC 401", async () => {
    const transport = new DirectAssistantTransport({
      fetch: vi.fn(async () =>
        new Response(JSON.stringify({ message: "upstream rejected" }), {
          status: 401,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    });
    const conversationId = await createPocConversation(transport);
    const events = await send(transport, conversationId, "preserve");

    expect(terminal(events)).toMatchObject({
      status: "failed",
      error: {
        code: "http_401",
        message: "The agent stream could not be started.",
      },
    });
    await Promise.resolve();
    expect(useAuthStore.getState().user?.id).toBe("agent-poc-user");
  });

  it("sends only user and assistant final text on later POC turns", async () => {
    const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
      void args;
      return sseResponse(happyFixture);
    });
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await createPocConversation(transport);

    await send(transport, conversationId, "first question");
    await send(transport, conversationId, "second question");

    const request = JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body)) as {
      messages: Array<{ role: string; content: string }>;
    };
    expect(request.messages).toEqual([
      { role: "user", content: "first question" },
      { role: "assistant", content: "Chrono Sandbox is healthy." },
      { role: "user", content: "second question" },
    ]);
    expect(JSON.stringify(request)).not.toContain("Check health");
    expect(JSON.stringify(request)).not.toContain("health_handler");
  });

  it.each([
    ["POC to ordinary", true, false],
    ["ordinary to POC", false, true],
  ])(
    "keeps mode switch %s request shapes isolated",
    async (_name, firstPoc, secondPoc) => {
      let call = 0;
      const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
        void args;
        const poc = call === 0 ? firstPoc : secondPoc;
        call += 1;
        return sseResponse(poc ? happyFixture : directFixture);
      });
      const transport = new DirectAssistantTransport({ fetch: fetchMock });
      const conversationId = (await transport.createConversation()).id;
      transport.setAgentPocMode(conversationId, firstPoc);

      await send(transport, conversationId, "first");
      transport.setAgentPocMode(conversationId, secondPoc);
      await send(transport, conversationId, "second");

      expect(fetchMock.mock.calls.map((entry) => entry[0])).toEqual([
        firstPoc
          ? DIRECT_AGENT_POC_URL
          : "/api/v1/assistant/direct/completions",
        secondPoc
          ? DIRECT_AGENT_POC_URL
          : "/api/v1/assistant/direct/completions",
      ]);
      const request = JSON.parse(
        String(fetchMock.mock.calls[1]?.[1]?.body),
      ) as {
        messages: Array<{ role: string; content: string }>;
      };
      expect(request.messages.at(-1)).toEqual({
        role: "user",
        content: "second",
      });
      expect(JSON.stringify(request)).not.toContain("Check health");
      expect(JSON.stringify(request)).not.toContain("health_handler");
    },
  );

  it("bounds POC content to 128 KiB without changing the ordinary cap", async () => {
    const fetchMock = vi.fn(async (...args: Parameters<typeof fetch>) => {
      void args;
      return sseResponse(happyFixture);
    });
    const transport = new DirectAssistantTransport({ fetch: fetchMock });
    const conversationId = await createPocConversation(transport);

    for (let index = 0; index < 5; index += 1) {
      await send(
        transport,
        conversationId,
        `${String(index)}:${"x".repeat(32_760)}`,
      );
    }

    const body = String(fetchMock.mock.calls.at(-1)?.[1]?.body);
    const request = JSON.parse(body) as {
      messages: Array<{ role: string; content: string }>;
    };
    const contentBytes = encoder.encode(
      request.messages.map((message) => message.content).join(""),
    ).byteLength;
    expect(contentBytes).toBeLessThanOrEqual(MAX_AGENT_POC_CONTENT_BYTES);
    expect(request.messages.at(-1)?.content.startsWith("4:")).toBe(true);
  });
});
