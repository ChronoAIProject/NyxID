import { create } from "zustand";
import { assistantPinsSchema } from "@/schemas/assistant-pins";

export const PIN_STORAGE_KEY = "nyxid.assistant_pins";
const EMPTY_IDS: readonly string[] = [];

interface AssistantPinsState {
  readonly ownerUserId: string | null;
  readonly ids: readonly string[];
  readonly error: "load" | "save" | "clear" | null;
  readonly bindAccount: (userId: string | null) => void;
  readonly toggle: (userId: string, conversationId: string) => void;
  readonly remove: (userId: string, conversationId: string) => void;
}

/** Browser preference only: persist IDs, never conversation content or authority. */
export function createAssistantPinsStore() {
  return create<AssistantPinsState>((set, get) => {
    function save(userId: string, ids: readonly string[]): boolean {
      try {
        if (ids.length) {
          const payload = assistantPinsSchema.parse({ version: 1, ownerUserId: userId, ids });
          localStorage.setItem(PIN_STORAGE_KEY, JSON.stringify(payload));
        } else {
          localStorage.removeItem(PIN_STORAGE_KEY);
        }
        set({ ids, error: null });
        return true;
      } catch {
        // The caller keeps the last saved UI state and surfaces a localized error.
        set({ error: "save" });
        return false;
      }
    }

    return {
      ownerUserId: null,
      ids: EMPTY_IDS,
      error: null,
      bindAccount: (userId) => {
        const previous = get();
        if (userId && previous.ownerUserId === userId && !previous.error) return;
        // Clear memory before reading storage, including when storage is unavailable.
        set({ ownerUserId: userId, ids: EMPTY_IDS, error: null });
        try {
          if (!userId || (previous.ownerUserId && previous.ownerUserId !== userId)) {
            localStorage.removeItem(PIN_STORAGE_KEY);
            return;
          }
          const raw = localStorage.getItem(PIN_STORAGE_KEY);
          if (!raw) return;
          const stored = assistantPinsSchema.safeParse(JSON.parse(raw));
          if (!stored.success) throw new Error("Invalid conversation pin preference");
          if (stored.data.ownerUserId !== userId) {
            localStorage.removeItem(PIN_STORAGE_KEY);
            return;
          }
          set({ ids: [...new Set(stored.data.ids)] });
        } catch {
          set({ error: userId ? "load" : "clear" });
        }
      },
      toggle: (userId, conversationId) => {
        if (!userId || get().ownerUserId !== userId || !conversationId) return;
        const ids = get().ids;
        save(userId, ids.includes(conversationId)
          ? ids.filter((id) => id !== conversationId)
          : [...ids, conversationId]);
      },
      remove: (userId, conversationId) => {
        if (get().ownerUserId !== userId || !get().ids.includes(conversationId)) return;
        const ids = get().ids.filter((id) => id !== conversationId);
        if (!save(userId, ids)) set({ ids });
      },
    };
  });
}

export const useAssistantPinsStore = createAssistantPinsStore();

/** Stable partition: retain the server's ordinary order in both groups. O(n). */
export function pinnedConversationsFirst<T extends { readonly id: string }>(
  conversations: readonly T[],
  pinnedIds: ReadonlySet<string>,
): T[] {
  const pinned: T[] = [];
  const other: T[] = [];
  for (const conversation of conversations) {
    (pinnedIds.has(conversation.id) ? pinned : other).push(conversation);
  }
  return [...pinned, ...other];
}
