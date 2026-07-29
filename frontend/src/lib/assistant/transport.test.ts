import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  assistantTransport,
  resetAssistantTransport,
  selectAssistantTransportKind,
} from "@/lib/assistant/transport";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import type { TurnEvent } from "@/types/assistant";

describe("selectAssistantTransportKind", () => {
  it("keeps vitest sessions on the scripted mock", () => {
    expect(
      selectAssistantTransportKind({ mode: "test", dev: false, search: "" }),
    ).toBe("mock");
  });

  it("uses the mock for dev sessions that opt in with ?mock", () => {
    expect(
      selectAssistantTransportKind({
        mode: "development",
        dev: true,
        search: "?mock",
      }),
    ).toBe("mock");
  });

  it("talks to aevatar for production and plain dev sessions", () => {
    expect(
      selectAssistantTransportKind({
        mode: "production",
        dev: false,
        search: "",
      }),
    ).toBe("aevatar");
    expect(
      selectAssistantTransportKind({
        mode: "development",
        dev: true,
        search: "",
      }),
    ).toBe("aevatar");
  });
});

describe("session transport", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetAssistantTransport(() => Date.parse("2026-07-29T00:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("never instantiates the aevatar transport for a vitest session", () => {
    // The test session resolves to the scripted mock, not the live transport.
    expect(assistantTransport).not.toBeInstanceOf(AevatarAssistantTransport);
  });

  it("demonstrates the action-card resolution and follow-up turn in the mock", async () => {
    const conversation = await assistantTransport.createConversation();
    const events: TurnEvent[] = [];

    assistantTransport.sendMessage(
      conversation.id,
      "Read my repositories",
      (event) => {
        events.push(event);
      },
    );
    await vi.advanceTimersByTimeAsync(1_300);

    const history = await assistantTransport.getHistory(conversation.id);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "action_card");
    if (!card || card.type !== "action_card") {
      throw new Error("mock action card was not emitted");
    }
    expect(card).toMatchObject({
      status: "pending",
      action_request_id: expect.stringMatching(/^act-turn-\d+$/),
      params: {
        variant: "catalog",
        service_slug: "api-github",
        requested_scopes: ["repo"],
      },
    });

    const continuation = assistantTransport.continueActions(
      conversation.id,
      card.origin_turn_id,
      [
        {
          actionRequestId: card.action_request_id,
          originTurnId: card.origin_turn_id,
          disposition: "completed",
          resource: {
            userService: { userServiceId: "mock-user-service-1" },
          },
        },
      ],
      (event) => events.push(event),
    );
    expect(continuation).not.toBeNull();
    await vi.advanceTimersByTimeAsync(700);

    const completedHistory = await assistantTransport.getHistory(
      conversation.id,
    );
    expect(
      completedHistory.messages
        .flatMap((message) => message.blocks)
        .find((block) => block.type === "action_card"),
    ).toMatchObject({ status: "completed" });
    expect(
      completedHistory.messages
        .flatMap((message) => message.blocks)
        .find(
          (block) =>
            block.type === "text" &&
            block.text.includes("service connection is ready"),
        ),
    ).toBeDefined();
    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "completed",
    });
  });
});
