import { describe, expect, it, vi } from "vitest";
import {
  AssistantEngineRouter,
  DelegatingAssistantTransport,
  createAssistantTransportForEnvironment,
} from "@/lib/assistant/transport";
import type {
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
} from "@/types/assistant";

class EngineProbe implements AssistantTransport {
  readonly calls: string[] = [];
  readonly handleCancel = vi.fn();
  private readonly label: string;

  constructor(label: string) {
    this.label = label;
  }

  async listConversations(): Promise<Conversation[]> {
    this.calls.push("list");
    return [];
  }

  async createConversation(): Promise<Conversation> {
    this.calls.push("create");
    return {
      id: `${this.label}-created`,
      title: this.label,
      created_at: "2026-08-11T00:00:00.000Z",
      last_message_at: "2026-08-11T00:00:00.000Z",
    };
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    this.calls.push(`history:${conversationId}`);
    return {
      conversation: {
        id: conversationId,
        title: this.label,
        created_at: "2026-08-11T00:00:00.000Z",
        last_message_at: "2026-08-11T00:00:00.000Z",
      },
      messages: [],
      has_more: false,
    };
  }

  async reconcileProjection(conversationId: string) {
    this.calls.push(`reconcile:${conversationId}`);
    return { status: "materialized" as const, conversationId };
  }

  releaseProjectionWaiter(conversationId: string): void {
    this.calls.push(`release:${conversationId}`);
  }

  async deleteConversation(conversationId: string): Promise<void> {
    this.calls.push(`delete:${conversationId}`);
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    void content;
    void onEvent;
    this.calls.push(`send:${conversationId}`);
    return { turnId: `${this.label}-turn`, cancel: this.handleCancel };
  }

  cancelActiveTurn(conversationId: string): void {
    this.calls.push(`cancel:${conversationId}`);
  }

  async stopTask(conversationId: string): Promise<void> {
    this.calls.push(`stop:${conversationId}`);
  }

  async steerTask(conversationId: string): Promise<void> {
    this.calls.push(`steer:${conversationId}`);
  }

  async retryStep(conversationId: string): Promise<void> {
    this.calls.push(`retry:${conversationId}`);
  }

  async skipStep(conversationId: string): Promise<void> {
    this.calls.push(`skip:${conversationId}`);
  }

  async resolvePlan(conversationId: string): Promise<void> {
    this.calls.push(`plan:${conversationId}`);
  }

  async decideApproval(): Promise<TurnHandle | null> {
    return null;
  }

  async resolveInput(): Promise<TurnHandle | null> {
    return null;
  }

  setActionCardInProgress(): void {}
  blockActionCard(): void {}
  continueActions(): TurnHandle | null {
    return null;
  }
  wakeActions(): TurnHandle {
    return { turnId: `${this.label}-wake`, cancel: vi.fn() };
  }
}

describe("AssistantEngineRouter", () => {
  it("routes engine-scoped calls from the selected flag state", async () => {
    const aevatar = new EngineProbe("aevatar");
    const direct = new EngineProbe("direct");
    const router = new AssistantEngineRouter(aevatar, direct);

    await router.listConversations();
    await router.createConversation();
    router.setSelectedEngine("direct");
    await router.listConversations();
    await router.createConversation();

    expect(aevatar.calls).toEqual(["list", "create"]);
    expect(direct.calls).toEqual(["list", "create"]);
  });

  it("routes conversation calls by validated prefix and fails closed", async () => {
    const aevatar = new EngineProbe("aevatar");
    const direct = new EngineProbe("direct");
    const router = new AssistantEngineRouter(aevatar, direct);
    router.setSelectedEngine("direct");

    await router.getHistory("nyxid-chat-server-1");
    await router.getHistory("chatc-server-2");
    await router.getHistory("workflow-pending-local-3");
    await router.getHistory("direct-local-4");

    expect(aevatar.calls).toEqual([
      "history:nyxid-chat-server-1",
      "history:chatc-server-2",
      "history:workflow-pending-local-3",
    ]);
    expect(direct.calls).toEqual(["history:direct-local-4"]);
    expect(() => router.getHistory("untrusted-id")).toThrow(
      "Unknown assistant conversation id",
    );
    expect(() => router.cancelActiveTurn("untrusted-id")).toThrow(
      "Unknown assistant conversation id",
    );
  });

  it("cancels through the originating delegate after a mid-stream flag flip", () => {
    const aevatar = new EngineProbe("aevatar");
    const direct = new EngineProbe("direct");
    const router = new AssistantEngineRouter(aevatar, direct);

    router.sendMessage("nyxid-chat-live-1", "hello", () => undefined);
    router.setSelectedEngine("direct");
    router.cancelActiveTurn("nyxid-chat-live-1");

    expect(aevatar.calls).toEqual([
      "send:nyxid-chat-live-1",
      "cancel:nyxid-chat-live-1",
    ]);
    expect(direct.calls).toEqual([]);
  });

  it("returns the delegate handle with its own cancellation semantics", () => {
    const aevatar = new EngineProbe("aevatar");
    const direct = new EngineProbe("direct");
    const router = new AssistantEngineRouter(aevatar, direct);

    const handle = router.sendMessage(
      "nyxid-chat-live-1",
      "hello",
      () => undefined,
    );
    router.setSelectedEngine("direct");
    handle.cancel();

    expect(aevatar.handleCancel).toHaveBeenCalledOnce();
    expect(aevatar.calls).toEqual(["send:nyxid-chat-live-1"]);
    expect(direct.calls).toEqual([]);
  });
});

describe("assistant transport environment layering", () => {
  const factories = () => ({
    createMock: () => new EngineProbe("mock"),
    createAevatar: () => new EngineProbe("aevatar"),
    createDirect: () => new EngineProbe("direct"),
  });

  it("uses the mock directly for Vitest and dev ?mock", () => {
    expect(
      createAssistantTransportForEnvironment(
        { mode: "test", dev: false, search: "" },
        factories(),
      ),
    ).toBeInstanceOf(EngineProbe);
    expect(
      createAssistantTransportForEnvironment(
        { mode: "development", dev: true, search: "?mock" },
        factories(),
      ),
    ).toBeInstanceOf(EngineProbe);
  });

  it("uses the router directly in production and inside the scenario slot in dev", () => {
    expect(
      createAssistantTransportForEnvironment(
        { mode: "production", dev: false, search: "" },
        factories(),
      ),
    ).toBeInstanceOf(AssistantEngineRouter);

    const dev = createAssistantTransportForEnvironment(
      { mode: "development", dev: true, search: "" },
      factories(),
    );
    expect(dev).toBeInstanceOf(DelegatingAssistantTransport);
    expect((dev as DelegatingAssistantTransport).current()).toBeInstanceOf(
      AssistantEngineRouter,
    );
  });
});
