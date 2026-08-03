import { beforeEach, describe, expect, it } from "vitest";
import {
  ASSISTANT_MOCK_SCENARIOS_STORAGE_KEY,
  useAssistantMockScenariosStore,
} from "./assistant-mock-scenarios-store";

function resetStore(): void {
  useAssistantMockScenariosStore.setState({
    enabled: false,
    disabledScenarioIds: [],
    world: { connected: [] },
    userId: null,
    engineState: "idle",
    lastActivity: null,
  });
}

describe("useAssistantMockScenariosStore", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStore();
  });

  it("persists only the versioned user-owned scenario settings and world", () => {
    const store = useAssistantMockScenariosStore.getState();
    store.ensureUser("user-a");
    store.setEnabled(true);
    store.setScenarioEnabled("github-issues", false);
    store.connectService("api-github");
    store.setEngineState("ready");
    store.noteActivity({
      scenarioId: "connect-github",
      matched: true,
      at: 123,
    });

    const persisted = JSON.parse(
      localStorage.getItem(ASSISTANT_MOCK_SCENARIOS_STORAGE_KEY)!,
    ) as { version: number; state: Record<string, unknown> };

    expect(persisted).toEqual({
      version: 1,
      state: {
        enabled: true,
        disabledScenarioIds: ["github-issues"],
        world: { connected: ["api-github"] },
        userId: "user-a",
      },
    });
    expect(persisted.state).not.toHaveProperty("engineState");
    expect(persisted.state).not.toHaveProperty("lastActivity");
  });

  it("updates scenario opt-outs and world membership without duplicates", () => {
    const store = useAssistantMockScenariosStore.getState();
    store.setScenarioEnabled("github-issues", false);
    store.setScenarioEnabled("github-issues", false);
    store.connectService("api-github");
    store.connectService("api-github");
    store.connectService("api-openai");

    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      disabledScenarioIds: ["github-issues"],
      world: { connected: ["api-github", "api-openai"] },
    });

    store.setScenarioEnabled("github-issues", true);
    store.disconnectService("api-github");
    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      disabledScenarioIds: [],
      world: { connected: ["api-openai"] },
    });

    store.resetWorld();
    expect(useAssistantMockScenariosStore.getState().world.connected).toEqual(
      [],
    );
  });

  it("reset restores every default", () => {
    const store = useAssistantMockScenariosStore.getState();
    store.ensureUser("user-a");
    store.setEnabled(true);
    store.setScenarioEnabled("error-demo", false);
    store.connectService("api-github");
    store.setEngineState("error");
    store.noteActivity({ scenarioId: null, matched: false, at: 456 });

    store.reset();

    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      enabled: false,
      disabledScenarioIds: [],
      world: { connected: [] },
      userId: null,
      engineState: "idle",
      lastActivity: null,
    });
  });

  it("resets a foreign persisted world on an account switch (F14)", async () => {
    localStorage.setItem(
      ASSISTANT_MOCK_SCENARIOS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        state: {
          enabled: true,
          disabledScenarioIds: ["approval-demo"],
          world: { connected: ["api-github"] },
          userId: "user-a",
        },
      }),
    );
    await useAssistantMockScenariosStore.persist.rehydrate();
    useAssistantMockScenariosStore.getState().setEngineState("ready");

    useAssistantMockScenariosStore.getState().ensureUser("user-b");

    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      enabled: false,
      disabledScenarioIds: [],
      world: { connected: [] },
      userId: "user-b",
      engineState: "ready",
      lastActivity: null,
    });
  });

  it("keeps state when ensureUser receives the current owner", () => {
    const store = useAssistantMockScenariosStore.getState();
    store.ensureUser("user-a");
    store.setEnabled(true);
    store.connectService("api-github");

    store.ensureUser("user-a");

    expect(useAssistantMockScenariosStore.getState()).toMatchObject({
      enabled: true,
      world: { connected: ["api-github"] },
      userId: "user-a",
    });
  });
});
