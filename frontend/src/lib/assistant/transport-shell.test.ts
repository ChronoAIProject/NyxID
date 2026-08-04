import { beforeEach, describe, expect, it, vi } from "vitest";
import { installScenarioInterceptor } from "@/lib/assistant/scenario-intercept-transport";
import {
  DelegatingAssistantTransport,
  applyAssistantScenarioFeature,
  createAssistantTransportForEnvironment,
  installAssistantTransportInterceptor,
  type AssistantInterceptorLoader,
  type AssistantInterceptorModule,
} from "@/lib/assistant/transport";
import * as storeModule from "@/stores/assistant-mock-scenarios-store";
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

  async decideApproval(): Promise<TurnHandle | null> {
    this.calls.push("approval");
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

/** Loader pair standing in for the two real async chunks. */
function testLoaders(interceptor: AssistantInterceptorLoader) {
  const loadStore = vi.fn(async () => storeModule);
  return { loaders: { loadInterceptor: interceptor, loadStore }, loadStore };
}

describe("DelegatingAssistantTransport flag-gated installation", () => {
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
      featureEnabled: false,
      enabled: false,
      disabledScenarioIds: [],
      world: { connected: [] },
      userId: null,
      engineState: "idle",
      lastActivity: null,
    });
  });

  it("loads nothing at all while the flag is off (never-armed tab)", async () => {
    const live = new RecordingTransport("live");
    const shell = new DelegatingAssistantTransport(live);
    const loadInterceptor = vi.fn(
      async (): Promise<AssistantInterceptorModule> => ({
        installScenarioInterceptor: () => undefined,
      }),
    );
    const { loaders, loadStore } = testLoaders(loadInterceptor);

    await applyAssistantScenarioFeature(false, shell, loaders);

    // Neither chunk is fetched: no interceptor, no persisted store, no
    // localStorage read — a flagless session pays nothing.
    expect(loadInterceptor).not.toHaveBeenCalled();
    expect(loadStore).not.toHaveBeenCalled();
    expect(shell.current()).toBe(live);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("idle");
  });

  it("delegates bare calls before load and intercepts in place after the flag arms it (P5)", async () => {
    const live = new RecordingTransport("live");
    const shell = new DelegatingAssistantTransport(live);
    let resolveLoader: (module: AssistantInterceptorModule) => void = () =>
      undefined;
    const loadInterceptor = vi.fn(
      () =>
        new Promise<AssistantInterceptorModule>((resolve) => {
          resolveLoader = resolve;
        }),
    );
    const { loaders } = testLoaders(loadInterceptor);

    const applied = applyAssistantScenarioFeature(true, shell, loaders);
    await vi.waitFor(() =>
      expect(useAssistantMockScenariosStore.getState().engineState).toBe(
        "loading",
      ),
    );
    expect(useAssistantMockScenariosStore.getState().featureEnabled).toBe(true);

    await shell.listConversations();
    expect(live.calls).toEqual(["list"]);
    resolveLoader({ installScenarioInterceptor });
    await applied;
    await shell.listConversations();

    expect(loadInterceptor).toHaveBeenCalledTimes(1);
    expect(shell.current()).not.toBe(live);
    expect(live.calls).toEqual(["list", "list"]);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("ready");
  });

  it("clears featureEnabled on an armed tab when the flag is revoked mid-session", async () => {
    const live = new RecordingTransport("live");
    const shell = new DelegatingAssistantTransport(live);
    const { loaders } = testLoaders(async () => ({
      installScenarioInterceptor,
    }));

    await applyAssistantScenarioFeature(true, shell, loaders);
    useAssistantMockScenariosStore.getState().setEnabled(true);
    expect(useAssistantMockScenariosStore.getState().featureEnabled).toBe(true);

    await applyAssistantScenarioFeature(false, shell, loaders);

    // The interceptor stays wrapped (nothing can unwrap it), so the revoked
    // flag has to reach it through the store — the user's own toggle is left
    // alone and is no longer sufficient to intercept.
    expect(shell.current()).not.toBe(live);
    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      featureEnabled: false,
      enabled: true,
    });
  });

  it("keeps the live delegate, reports error, and allows a retry when the chunk fails (P5, F6)", async () => {
    const live = new RecordingTransport("live");
    const shell = new DelegatingAssistantTransport(live);
    const loadInterceptor = vi
      .fn<AssistantInterceptorLoader>()
      .mockRejectedValueOnce(new Error("chunk failed"))
      .mockResolvedValueOnce({ installScenarioInterceptor });
    const { loaders } = testLoaders(loadInterceptor);

    await applyAssistantScenarioFeature(true, shell, loaders);
    await shell.listConversations();

    expect(shell.current()).toBe(live);
    expect(live.calls).toEqual(["list"]);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("error");

    // A failed load must not poison the shell for the rest of the tab.
    await applyAssistantScenarioFeature(true, shell, loaders);
    expect(loadInterceptor).toHaveBeenCalledTimes(2);
    expect(shell.current()).not.toBe(live);
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("ready");
  });

  it("never throws into the caller when the store chunk fails to load", async () => {
    const shell = new DelegatingAssistantTransport(
      new RecordingTransport("live"),
    );
    const loadInterceptor = vi.fn(
      async (): Promise<AssistantInterceptorModule> => ({
        installScenarioInterceptor: () => undefined,
      }),
    );

    await expect(
      applyAssistantScenarioFeature(true, shell, {
        loadInterceptor,
        loadStore: async () => {
          throw new Error("store chunk failed");
        },
      }),
    ).resolves.toBeUndefined();
    expect(loadInterceptor).not.toHaveBeenCalled();
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

  it("never wraps a full-mock transport, flag on or off (P5)", async () => {
    const mock = new RecordingTransport("mock");
    const loadInterceptor = vi.fn(
      async (): Promise<AssistantInterceptorModule> => ({
        installScenarioInterceptor: () => undefined,
      }),
    );
    const { loaders, loadStore } = testLoaders(loadInterceptor);
    const factories = {
      createMock: () => mock,
      createAevatar: () => new RecordingTransport("live"),
    };

    const fullMock = createAssistantTransportForEnvironment(
      { mode: "test", dev: false, search: "" },
      factories,
    );
    expect(fullMock).toBe(mock);

    await applyAssistantScenarioFeature(true, fullMock, loaders);

    expect(loadInterceptor).not.toHaveBeenCalled();
    expect(loadStore).not.toHaveBeenCalled();
  });

  it("returns a bare shell for every environment — dev no longer installs (P5)", () => {
    const factories = {
      createMock: () => new RecordingTransport("mock"),
      createAevatar: () => new RecordingTransport("live"),
    };
    for (const env of [
      { mode: "development", dev: true, search: "" },
      { mode: "production", dev: false, search: "" },
    ]) {
      const transport = createAssistantTransportForEnvironment(env, factories);
      expect(transport).toBeInstanceOf(DelegatingAssistantTransport);
    }
    expect(useAssistantMockScenariosStore.getState().engineState).toBe("idle");
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
