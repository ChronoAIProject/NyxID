import { create } from "zustand";
import { persist } from "zustand/middleware";

const STORAGE_KEY = "nyxid.assistant_context";
const MAX_BINDINGS = 50;

export interface AssistantScreenBinding {
  readonly conversationId: string;
  readonly updatedAt: number;
}

interface AssistantContextState {
  readonly ownerUserId: string | null;
  readonly lastScreen: string | null;
  readonly bindings: Record<string, AssistantScreenBinding>;
  readonly recordScreen: (userId: string, screenKey: string) => void;
  readonly bindConversation: (
    userId: string,
    screenKey: string,
    conversationId: string,
  ) => void;
  readonly pruneBindings: (existingConversationIds: readonly string[]) => void;
  readonly clear: () => void;
}

const EMPTY_CONTEXT = {
  ownerUserId: null,
  lastScreen: null,
  bindings: {},
} as const;

function newestBindings(
  bindings: Record<string, AssistantScreenBinding>,
  retainedScreenKey: string,
): Record<string, AssistantScreenBinding> {
  return Object.fromEntries(
    Object.entries(bindings)
      .sort(([leftKey, left], [rightKey, right]) => {
        const recency = right.updatedAt - left.updatedAt;
        if (recency !== 0) return recency;
        if (leftKey === retainedScreenKey) return -1;
        if (rightKey === retainedScreenKey) return 1;
        return 0;
      })
      .slice(0, MAX_BINDINGS),
  );
}

export const useAssistantContextStore = create<AssistantContextState>()(
  persist(
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
      bindConversation: (userId, screenKey, conversationId) => {
        set((state) => {
          const owned =
            state.ownerUserId === userId
              ? state
              : { ...EMPTY_CONTEXT, ownerUserId: userId };
          return {
            ownerUserId: userId,
            lastScreen: owned.lastScreen,
            bindings: newestBindings(
              {
                ...owned.bindings,
                [screenKey]: { conversationId, updatedAt: Date.now() },
              },
              screenKey,
            ),
          };
        });
      },
      pruneBindings: (existingConversationIds) => {
        const existing = new Set(existingConversationIds);
        set((state) => ({
          bindings: Object.fromEntries(
            Object.entries(state.bindings).filter(([, binding]) =>
              existing.has(binding.conversationId),
            ),
          ),
        }));
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
      version: 1,
      partialize: ({ ownerUserId, lastScreen, bindings }) => ({
        ownerUserId,
        lastScreen,
        bindings,
      }),
    },
  ),
);
