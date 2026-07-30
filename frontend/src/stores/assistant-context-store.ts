import { create } from "zustand";
import { persist } from "zustand/middleware";

const STORAGE_KEY = "nyxid.assistant_context";

interface AssistantContextState {
  readonly ownerUserId: string | null;
  readonly lastScreen: string | null;
  readonly recordScreen: (userId: string, screenKey: string) => void;
  readonly clear: () => void;
}

type PersistedAssistantContext = Pick<
  AssistantContextState,
  "ownerUserId" | "lastScreen"
>;

const EMPTY_CONTEXT = {
  ownerUserId: null,
  lastScreen: null,
} as const;

function migrateAssistantContext(
  persistedState: unknown,
): PersistedAssistantContext {
  if (typeof persistedState !== "object" || persistedState === null) {
    return EMPTY_CONTEXT;
  }
  const state = persistedState as Record<string, unknown>;
  return {
    ownerUserId:
      typeof state.ownerUserId === "string" ? state.ownerUserId : null,
    lastScreen: typeof state.lastScreen === "string" ? state.lastScreen : null,
  };
}

export const useAssistantContextStore = create<AssistantContextState>()(
  persist<AssistantContextState, [], [], PersistedAssistantContext>(
    (set) => ({
      ...EMPTY_CONTEXT,
      recordScreen: (userId, screenKey) => {
        set((state) => {
          if (state.ownerUserId !== userId) {
            return {
              ...EMPTY_CONTEXT,
              ownerUserId: userId,
              lastScreen: screenKey,
            };
          }
          return { lastScreen: screenKey };
        });
      },
      clear: () => {
        set(EMPTY_CONTEXT);
        if (typeof localStorage !== "undefined") {
          localStorage.removeItem(STORAGE_KEY);
        }
      },
    }),
    {
      name: STORAGE_KEY,
      version: 2,
      migrate: migrateAssistantContext,
      partialize: ({ ownerUserId, lastScreen }) => ({
        ownerUserId,
        lastScreen,
      }),
    },
  ),
);
