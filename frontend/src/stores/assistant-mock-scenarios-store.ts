import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

export const ASSISTANT_MOCK_SCENARIOS_STORAGE_KEY =
  "nyxid.assistant.mockscenarios.v1";

export type MockScenarioEngineState = "idle" | "loading" | "ready" | "error";

export interface MockScenarioWorld {
  readonly connected: readonly string[];
}

export interface MockScenarioActivity {
  readonly scenarioId: string | null;
  readonly matched: boolean;
  readonly at: number;
}

interface AssistantMockScenariosState {
  readonly enabled: boolean;
  readonly disabledScenarioIds: readonly string[];
  readonly world: MockScenarioWorld;
  readonly userId: string | null;
  readonly engineState: MockScenarioEngineState;
  readonly lastActivity: MockScenarioActivity | null;
  readonly setEnabled: (enabled: boolean) => void;
  readonly setScenarioEnabled: (scenarioId: string, enabled: boolean) => void;
  readonly setEngineState: (engineState: MockScenarioEngineState) => void;
  readonly connectService: (serviceSlug: string) => void;
  readonly disconnectService: (serviceSlug: string) => void;
  readonly resetWorld: () => void;
  readonly noteActivity: (activity: MockScenarioActivity) => void;
  readonly ensureUser: (userId: string) => void;
  readonly reset: () => void;
}

const DEFAULT_STATE = {
  enabled: false,
  disabledScenarioIds: [] as readonly string[],
  world: { connected: [] as readonly string[] },
  userId: null,
  engineState: "idle" as const,
  lastActivity: null,
};

export const useAssistantMockScenariosStore =
  create<AssistantMockScenariosState>()(
    persist(
      (set) => ({
        ...DEFAULT_STATE,
        setEnabled: (enabled) => set({ enabled }),
        setScenarioEnabled: (scenarioId, enabled) =>
          set((state) => ({
            disabledScenarioIds: enabled
              ? state.disabledScenarioIds.filter((id) => id !== scenarioId)
              : state.disabledScenarioIds.includes(scenarioId)
                ? state.disabledScenarioIds
                : [...state.disabledScenarioIds, scenarioId],
          })),
        setEngineState: (engineState) => set({ engineState }),
        connectService: (serviceSlug) =>
          set((state) => ({
            world: state.world.connected.includes(serviceSlug)
              ? state.world
              : { connected: [...state.world.connected, serviceSlug] },
          })),
        disconnectService: (serviceSlug) =>
          set((state) => ({
            world: {
              connected: state.world.connected.filter(
                (slug) => slug !== serviceSlug,
              ),
            },
          })),
        resetWorld: () => set({ world: DEFAULT_STATE.world }),
        noteActivity: (lastActivity) => set({ lastActivity }),
        ensureUser: (userId) =>
          set((state) =>
            state.userId === userId
              ? state
              : {
                  ...DEFAULT_STATE,
                  userId,
                  engineState: state.engineState,
                },
          ),
        reset: () => set(DEFAULT_STATE),
      }),
      {
        name: ASSISTANT_MOCK_SCENARIOS_STORAGE_KEY,
        version: 1,
        storage: createJSONStorage(() => localStorage),
        partialize: ({ enabled, disabledScenarioIds, world, userId }) => ({
          enabled,
          disabledScenarioIds,
          world,
          userId,
        }),
      },
    ),
  );
