import { beforeEach, describe, expect, it, vi } from "vitest";
import { installScenarioInterceptor } from "@/lib/assistant/scenario-intercept-transport";
import {
  AssistantEngineRouter,
  DelegatingAssistantTransport,
  createAssistantTransportForEnvironment,
  installAssistantTransportInterceptor,
  type AssistantInterceptorModule,
} from "@/lib/assistant/transport";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type {
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnHandle,
} from "@/types/assistant";

class RecordingTransport implements AssistantTransport {
  readonly calls: string[] = [];
  readonly label: string;

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
      id: `${this.label}-conversation`,
      title: this.label,
      created_at: "2026-08-04T00:00:00.000Z",
      last_message_at: "2026-08-04T00:00:00.000Z",
    };
  }

  async reconcileProjection(conversationId: string) {
    this.calls.push("reconcile");
    return { status: "materialized" as const, conversationId };
  }

  releaseProjectionWaiter(conversationId: string): void {
    void conversationId;
    this.calls.push("release");
  }

  async getHistory(): Promise<ConversationHistory> {
    this.calls.push("history");
    return {
      conversation: await this.createConversation(),
      messages: [],
      has_more: false,
    };
  }

  async deleteConversation(): Promise<void> {
    this.calls.push("delete");
  }

  sendMessage(): TurnHandle {
    this.calls.push("send");
    return { turnId: `${this.label}-turn`, cancel: () => undefined };
  }

  cancelActiveTurn(): void {
    this.calls.push("cancel");
  }

  async stopTask(): Promise<void> {
    this.calls.push("stop-task");
  }

  async steerTask(): Promise<void> {
    this.calls.push("steer-task");
  }

  async retryStep(): Promise<void> {
    this.calls.push("retry-step");
  }

  async skipStep(): Promise<void> {
    this.calls.push("skip-step");
  }

  async resolvePlan(): Promise<void> {
    this.calls.push("resolve-plan");
  }

  async decideApproval(): Promise<TurnHandle | null> {
    this.calls.push("approval");
    return null;
  }

  async resolveInput(): Promise<TurnHandle | null> {
    this.calls.push("input");
    return null;
  }

  setActionCardInProgress(): void {
    this.calls.push("progress");
  }

  blockActionCard(): void {
    this.calls.push("block");
  }

  continueActions(): TurnHandle | null {
    this.calls.push("continue");
    return null;
  }

  wakeActions(): TurnHandle {
    this.calls.push("wake");
    return { turnId: `${this.label}-wake`, cancel: () => undefined };
  }

}

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
    created_at: "2026-08-04T00:00:00.000Z",
  };
}

describe("DelegatingAssistantTransport dev installation", () => {
  beforeEach(() => {
    localStorage.clear();
    useAuthStore.setState({
      user: null,
      isAuthenticated: false,
      isLoading: false,
      mfaRequired: false,
      mfaToken: null,
    });
    useAssistantMockScenariosStore.setState({
      enabled: false,
      disabledScenarioIds: [],
      world: { connected: [] },
      userId: null,
      engineState: "idle",
      lastActivity: null,
    });
  });

  it("delegates bare calls before load and intercepts in place after install (P5)", async () => {
    const live = new RecordingTransport("live");
    let resolveLoader: (module: AssistantInterceptorModule) => void = () =>
      undefined;
    const loader = vi.fn(
      () =>
        new Promise<AssistantInterceptorModule>((resolve) => {
          resolveLoader = resolve;
        }),
    );
    const transport = createAssistantTransportForEnvironment(
      { mode: "development", dev: true, search: "" },
      {
        createMock: () => new RecordingTransport("mock"),
        createAevatar: () => live,
        createDirect: () => new RecordingTransport("direct"),
      },
      loader,
      (state) =>
        useAssistantMockScenariosStore.getState().setEngineState(state),
    );
    expect(transport).toBeInstanceOf(DelegatingAssistantTransport);
    const shell = transport as DelegatingAssistantTransport;
    expect(useAssistantMockScenariosStore.getState().engineState).toBe(
      "loading",
    );

    await shell.listConversations();
    expect(live.calls).toEqual(["list"]);
    resolveLoader({ installScenarioInterceptor });
    await installAssistantTransportInterceptor(shell, loader);
    await shell.listConversations();

    expect(loader).toHaveBeenCalledTimes(1);
    expect(shell.current()).not.toBe(live);
    expect(live.calls).toEqual(["list", "list"]);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("ready");
  });

  it("keeps the live delegate and reports error when dev installation fails (P5, F6)", async () => {
    const live = new RecordingTransport("live");
    let rejectLoader: (reason: Error) => void = () => undefined;
    const loader = vi.fn(
      () =>
        new Promise<AssistantInterceptorModule>((_resolve, reject) => {
          rejectLoader = reject;
        }),
    );
    const transport = createAssistantTransportForEnvironment(
      { mode: "development", dev: true, search: "" },
      {
        createMock: () => new RecordingTransport("mock"),
        createAevatar: () => live,
        createDirect: () => new RecordingTransport("direct"),
      },
      loader,
      (state) =>
        useAssistantMockScenariosStore.getState().setEngineState(state),
    );
    const shell = transport as DelegatingAssistantTransport;

    expect(useAssistantMockScenariosStore.getState().engineState).toBe(
      "loading",
    );
    rejectLoader(new Error("chunk failed"));

    await expect(
      installAssistantTransportInterceptor(shell, loader),
    ).rejects.toThrow("chunk failed");
    await shell.listConversations();

    expect(shell.current()).toBeInstanceOf(AssistantEngineRouter);
    expect(live.calls).toEqual(["list"]);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("error");
  });

  it("loads and installs at most once per shell (P5)", async () => {
    const shell = new DelegatingAssistantTransport(
      new RecordingTransport("live"),
    );
    const intercepted = new RecordingTransport("intercepted");
    const loader = vi.fn(
      async (): Promise<AssistantInterceptorModule> => ({
        installScenarioInterceptor: (target) => {
          target.install(intercepted);
        },
      }),
    );

    await Promise.all([
      installAssistantTransportInterceptor(shell, loader),
      installAssistantTransportInterceptor(shell, loader),
    ]);

    expect(loader).toHaveBeenCalledTimes(1);
    expect(shell.current()).toBe(intercepted);
  });

  it("never loads the interceptor for full-mock or production selection (P5)", () => {
    const mock = new RecordingTransport("mock");
    const loader = vi.fn(
      async (): Promise<AssistantInterceptorModule> => ({
        installScenarioInterceptor: () => undefined,
      }),
    );
    const factories = {
      createMock: () => mock,
      createAevatar: () => new RecordingTransport("live"),
      createDirect: () => new RecordingTransport("direct"),
    };

    const fullMock = createAssistantTransportForEnvironment(
      { mode: "test", dev: false, search: "" },
      factories,
      loader,
    );
    createAssistantTransportForEnvironment(
      { mode: "production", dev: false, search: "" },
      factories,
      loader,
    );

    expect(fullMock).toBe(mock);
    expect(loader).not.toHaveBeenCalled();
  });

  it("rescopes null -> A -> logout -> B in one module lifetime (P6, F14)", () => {
    const shell = new DelegatingAssistantTransport(
      new RecordingTransport("live"),
    );
    installScenarioInterceptor(shell);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("ready");

    useAuthStore.setState({ user: user("user-a"), isAuthenticated: true });
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("ready");
    useAssistantMockScenariosStore.getState().setEnabled(true);
    useAssistantMockScenariosStore.getState().connectService("api-github");
    useAuthStore.setState({ user: null, isAuthenticated: false });
    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      userId: "user-a",
      enabled: true,
      world: { connected: ["api-github"] },
      engineState: "ready",
    });

    useAuthStore.setState({ user: user("user-b"), isAuthenticated: true });

    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      userId: "user-b",
      enabled: false,
      world: { connected: [] },
      engineState: "ready",
    });
  });
});
