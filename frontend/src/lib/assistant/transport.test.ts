import { afterEach, describe, expect, it, vi } from "vitest";
import {
  assistantTransport,
  ModeSwitchingTransport,
} from "@/lib/assistant/transport";
import { useAssistantTransportStore } from "@/stores/assistant-transport-store";
import type {
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnHandle,
} from "@/types/assistant";

function conversation(id: string): Conversation {
  const at = new Date(0).toISOString();
  return { id, title: id, created_at: at, last_message_at: at };
}

function fakeTransport(tag: string) {
  const handle: TurnHandle = { turnId: `${tag}-turn`, cancel: vi.fn() };
  const transport = {
    listConversations: vi.fn(
      (): Promise<Conversation[]> => Promise.resolve([conversation(tag)]),
    ),
    createConversation: vi.fn(
      (): Promise<Conversation> => Promise.resolve(conversation(tag)),
    ),
    getHistory: vi.fn(
      (id: string): Promise<ConversationHistory> =>
        Promise.resolve({
          conversation: conversation(id),
          messages: [],
          has_more: false,
        }),
    ),
    sendMessage: vi.fn((): TurnHandle => handle),
    decideApproval: vi.fn((): Promise<void> => Promise.resolve()),
  };
  return { transport: transport satisfies AssistantTransport, handle };
}

afterEach(() => {
  useAssistantTransportStore.setState({ mode: "chat" });
});

describe("ModeSwitchingTransport", () => {
  it("routes every call to the transport the store selects, per call", async () => {
    const chat = fakeTransport("chat");
    const completions = fakeTransport("completions");
    const delegator = new ModeSwitchingTransport(
      chat.transport,
      completions.transport,
    );

    useAssistantTransportStore.setState({ mode: "completions" });
    expect(await delegator.listConversations()).toEqual([
      conversation("completions"),
    ]);
    await delegator.createConversation();
    await delegator.getHistory("completions-1");
    delegator.sendMessage("completions-1", "Hi", () => {});
    await delegator.decideApproval("completions-1", "block-1", true);
    expect(completions.transport.listConversations).toHaveBeenCalledTimes(1);
    expect(completions.transport.createConversation).toHaveBeenCalledTimes(1);
    expect(completions.transport.getHistory).toHaveBeenCalledWith(
      "completions-1",
    );
    expect(completions.transport.sendMessage).toHaveBeenCalledTimes(1);
    expect(completions.transport.decideApproval).toHaveBeenCalledWith(
      "completions-1",
      "block-1",
      true,
    );
    expect(chat.transport.listConversations).not.toHaveBeenCalled();
    expect(chat.transport.sendMessage).not.toHaveBeenCalled();

    // Flipping back re-targets without rebuilding the delegator.
    useAssistantTransportStore.setState({ mode: "chat" });
    expect(await delegator.listConversations()).toEqual([conversation("chat")]);
    delegator.sendMessage("nyxid-chat-1", "Hi", () => {});
    expect(chat.transport.sendMessage).toHaveBeenCalledTimes(1);
    expect(completions.transport.sendMessage).toHaveBeenCalledTimes(1);
  });

  it("routes to the workflow transport when the store selects it", async () => {
    const chat = fakeTransport("chat");
    const completions = fakeTransport("completions");
    const workflow = fakeTransport("workflow");
    const delegator = new ModeSwitchingTransport(
      chat.transport,
      completions.transport,
      workflow.transport,
    );

    useAssistantTransportStore.setState({ mode: "workflow" });
    expect(await delegator.listConversations()).toEqual([
      conversation("workflow"),
    ]);
    delegator.sendMessage("workflow-1", "Hi", () => {});
    expect(workflow.transport.sendMessage).toHaveBeenCalledTimes(1);
    expect(chat.transport.sendMessage).not.toHaveBeenCalled();
    expect(completions.transport.sendMessage).not.toHaveBeenCalled();
  });

  it("hands back the handle of the transport that started the turn", () => {
    const chat = fakeTransport("chat");
    const completions = fakeTransport("completions");
    const delegator = new ModeSwitchingTransport(
      chat.transport,
      completions.transport,
    );

    useAssistantTransportStore.setState({ mode: "completions" });
    const handle = delegator.sendMessage("completions-1", "Hi", () => {});
    // A mid-stream toggle must not misroute the cancel.
    useAssistantTransportStore.setState({ mode: "chat" });
    handle.cancel();

    expect(completions.handle.cancel).toHaveBeenCalledTimes(1);
    expect(chat.handle.cancel).not.toHaveBeenCalled();
  });
});

describe("session transport selection", () => {
  it("keeps vitest sessions on the scripted mock regardless of the store", () => {
    useAssistantTransportStore.setState({ mode: "completions" });
    expect(assistantTransport).not.toBeInstanceOf(ModeSwitchingTransport);
  });
});
