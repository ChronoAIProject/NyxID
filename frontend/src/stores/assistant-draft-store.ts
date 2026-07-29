import { create } from "zustand";
import { persist } from "zustand/middleware";

const STORAGE_KEY = "nyxid.assistant_drafts";
const MAX_DRAFTS = 50;

export interface AssistantDraft {
  readonly text: string;
  readonly updatedAt: number;
}

interface AssistantDraftState {
  readonly ownerUserId: string | null;
  readonly drafts: Record<string, AssistantDraft>;
  readonly saveDraft: (userId: string, key: string, text: string) => void;
  readonly getDraft: (key: string) => string;
  readonly clearDraft: (userId: string, key: string) => void;
  readonly pruneConversationDrafts: (
    existingConversationIds: readonly string[],
  ) => void;
  readonly clear: () => void;
}

const EMPTY_DRAFTS = {
  ownerUserId: null,
  drafts: {},
} as const;

function newestDrafts(
  drafts: Record<string, AssistantDraft>,
  retainedDraftKey: string,
): Record<string, AssistantDraft> {
  return Object.fromEntries(
    Object.entries(drafts)
      .sort(([leftKey, left], [rightKey, right]) => {
        const recency = right.updatedAt - left.updatedAt;
        if (recency !== 0) return recency;
        if (leftKey === retainedDraftKey) return -1;
        if (rightKey === retainedDraftKey) return 1;
        return 0;
      })
      .slice(0, MAX_DRAFTS),
  );
}

// Drafts are plaintext localStorage with last-writer-wins cross-tab behavior,
// matching the trust and synchronization model of existing local preferences.
export const useAssistantDraftStore = create<AssistantDraftState>()(
  persist(
    (set, get) => ({
      ...EMPTY_DRAFTS,
      saveDraft: (userId, key, text) => {
        set((state) => {
          const ownedDrafts: Record<string, AssistantDraft> =
            state.ownerUserId === userId ? state.drafts : EMPTY_DRAFTS.drafts;
          const drafts = { ...ownedDrafts };
          if (text.trim()) {
            drafts[key] = { text, updatedAt: Date.now() };
          } else {
            delete drafts[key];
          }
          return {
            ownerUserId: userId,
            drafts: newestDrafts(drafts, key),
          };
        });
      },
      getDraft: (key) => get().drafts[key]?.text ?? "",
      clearDraft: (userId, key) => {
        set((state) => {
          const drafts =
            state.ownerUserId === userId ? { ...state.drafts } : {};
          delete drafts[key];
          return { ownerUserId: userId, drafts };
        });
      },
      pruneConversationDrafts: (existingConversationIds) => {
        const existing = new Set(existingConversationIds);
        set((state) => ({
          drafts: Object.fromEntries(
            Object.entries(state.drafts).filter(([key]) => {
              if (!key.startsWith("conv:")) return true;
              return existing.has(key.slice("conv:".length));
            }),
          ),
        }));
      },
      clear: () => {
        set(EMPTY_DRAFTS);
        if (typeof localStorage !== "undefined") {
          localStorage.removeItem(STORAGE_KEY);
        }
      },
    }),
    {
      name: STORAGE_KEY,
      version: 1,
      partialize: ({ ownerUserId, drafts }) => ({ ownerUserId, drafts }),
    },
  ),
);
