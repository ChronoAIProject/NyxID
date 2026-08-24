import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readChatStreamFrames } from "@/lib/assistant/chat-api";
import {
  AssistantHttpFixtureWorld,
  type AssistantHttpFixtureFaults,
} from "@/lib/assistant/assistant-http-fixtures";
import { FIXTURE_REPLY } from "@/lib/assistant/assistant-http-fixture-data";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";

async function request(
  world: AssistantHttpFixtureWorld,
  endpoint: string,
  method = "GET",
  body?: unknown,
): Promise<Response> {
  const response = await world.handler({
    endpoint,
    init: {
      method,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    },
  });
  if (!response) throw new Error(`Fixture did not handle ${method} ${endpoint}`);
  return response;
}

describe("assistant HTTP fixture world", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantMockScenariosStore.getState().reset();
    globalThis.__nyxidAssistantHttpFaults = undefined;
    vi.useFakeTimers();
  });

  afterEach(() => {
    globalThis.__nyxidAssistantHttpFaults = undefined;
    vi.useRealTimers();
  });

  it("serves strict index, transcript, state, and idempotent delete responses", async () => {
    const world = new AssistantHttpFixtureWorld();
    const index = (await (await request(world, "/assistant/conversations")).json()) as {
      conversations: { id: string }[];
      nextCursor?: string;
    };
    expect(index.conversations.map((entry) => entry.id)).toContain(
      "conversation-stripe",
    );
    expect(index).not.toHaveProperty("nextCursor");

    const transcript = await (
      await request(world, "/assistant/conversations/conversation-stripe")
    ).json();
    expect(transcript).toMatchObject({
      projectionStatus: "current",
      stateVersion: 3,
      messages: expect.arrayContaining([
        expect.objectContaining({ role: "assistant", status: "completed" }),
      ]),
    });
    const state = await (
      await request(world, "/assistant/conversations/conversation-stripe/state")
    ).json();
    expect(state).toMatchObject({
      status: "current",
      stateVersion: 3,
      snapshot: {
        actorId: "conversation-stripe",
        pendingApproval: { approvalRequestId: "approval-request-lark-1" },
      },
    });

    expect(
      (await request(world, "/assistant/conversations/conversation-stripe", "DELETE")).status,
    ).toBe(204);
    expect(
      (await request(world, "/assistant/conversations/conversation-stripe", "DELETE")).status,
    ).toBe(204);
  });

  it("streams mixed-boundary SSE, skips malformed data, and persists the turn", async () => {
    const world = new AssistantHttpFixtureWorld();
    const response = await request(world, "/assistant/chat", "POST", {
      type: "text",
      clientRequestId: "request-1",
      prompt: "Inspect connected services",
    });
    const captured = response.text();
    await vi.advanceTimersByTimeAsync(10_000);
    const frames = [];
    for await (const frame of readChatStreamFrames(
      new Response(await captured, {
        headers: { "Content-Type": "text/event-stream" },
      }),
    )) {
      frames.push(frame);
    }
    expect(frames.some((frame) => frame.event?.type === "RUN_STARTED")).toBe(true);
    expect(frames.some((frame) => frame.event?.type === "RUN_FINISHED")).toBe(true);
    expect(
      frames.some(
        (frame) =>
          (frame.raw as { custom?: { name?: string } }).custom?.name ===
          "nyxid.action.request",
      ),
    ).toBe(true);

    const index = (await (await request(world, "/assistant/conversations")).json()) as {
      conversations: { id: string; messageCount: number }[];
    };
    const created = index.conversations.find((entry) =>
      entry.id.startsWith("nyxid-chat-mock-"),
    );
    expect(created?.messageCount).toBe(2);
    const transcript = (await (
      await request(world, `/assistant/conversations/${created?.id ?? "missing"}`)
    ).json()) as { messages: { content: string }[] };
    expect(transcript.messages.at(-1)?.content).toBe(FIXTURE_REPLY);
  });

  it("applies control mutations and injected state-envelope sequences", async () => {
    const world = new AssistantHttpFixtureWorld();
    expect(
      (
        await request(world, "/assistant/chat", "POST", {
          type: "approval.resolve",
          conversationId: "conversation-stripe",
          requestId: "approval-request-lark-1",
          approved: true,
        })
      ).status,
    ).toBe(202);
    const state = await (
      await request(world, "/assistant/conversations/conversation-stripe/state")
    ).json();
    expect(state).toMatchObject({
      stateVersion: 4,
      snapshot: {
        pendingApproval: null,
        latestApprovalResolution: { outcome: "approved" },
      },
    });

    const sequence: AssistantHttpFixtureFaults["stateEnvelopeSequence"] = [
      { status: "not_modified", stateVersion: 4 },
      { status: "reload_required" },
    ];
    globalThis.__nyxidAssistantHttpFaults = { stateEnvelopeSequence: sequence };
    expect(
      await (
        await request(world, "/assistant/conversations/conversation-stripe/state")
      ).json(),
    ).toEqual(sequence[0]);
    expect(
      await (
        await request(world, "/assistant/conversations/conversation-stripe/state")
      ).json(),
    ).toEqual(sequence[1]);
  });

  it("attributes coded and uncoded 401 fixture responses distinctly", async () => {
    const world = new AssistantHttpFixtureWorld();
    globalThis.__nyxidAssistantHttpFaults = {
      historyErrorStatus: 401,
      unauthorized: "coded",
    };
    expect(
      await (await request(world, "/assistant/conversations")).json(),
    ).toMatchObject({ error_code: 1001 });
    globalThis.__nyxidAssistantHttpFaults = {
      historyErrorStatus: 401,
      unauthorized: "uncoded",
    };
    expect(
      await (await request(world, "/assistant/conversations")).json(),
    ).toMatchObject({ error_code: 0 });
  });

  it("matches enabled HTTP scenarios and updates their connected fixture world", async () => {
    const world = new AssistantHttpFixtureWorld();
    useAssistantMockScenariosStore.setState({
      enabled: true,
      disabledScenarioIds: [],
      world: { connected: [] },
    });
    const response = await request(world, "/assistant/chat", "POST", {
      type: "text",
      clientRequestId: "scenario-request",
      prompt: "connect to my github",
    });
    const consumed = response.text();
    await vi.advanceTimersByTimeAsync(10_000);
    await consumed;

    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      lastActivity: { matched: true, scenarioId: "connect-github" },
      world: { connected: ["api-github"] },
    });
  });
});
