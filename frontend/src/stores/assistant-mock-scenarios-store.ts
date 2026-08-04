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
  /**
   * Server-resolved `experimental:assistant-mock-scenarios` value, mirrored
   * here by the React gate so the non-React interceptor can read it
   * synchronously (same bridge the wire-log panel uses). Never persisted —
   * it is platform-admin state, not a browser preference — and fail-closed at
   * `false` so a tab that has not yet resolved the flag cannot intercept.
   */
  readonly featureEnabled: boolean;
  readonly enabled: boolean;
  readonly disabledScenarioIds: readonly string[];
  readonly world: MockScenarioWorld;
  readonly userId: string | null;
  readonly engineState: MockScenarioEngineState;
  readonly lastActivity: MockScenarioActivity | null;
  readonly setFeatureEnabled: (featureEnabled: boolean) => void;
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
  featureEnabled: false,
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
        setFeatureEnabled: (featureEnabled) => set({ featureEnabled }),
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
                  // Session-scoped, not user-scoped: the engine chunk stays
                  // loaded and the flag stays as the gate last resolved it
                  // across a rescope.
                  engineState: state.engineState,
                  featureEnabled: state.featureEnabled,
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
