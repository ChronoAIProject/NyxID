/**
 * The shared preview column's width across chats and the threads where the
 * user hid the machine card. Width is shared; hidden thread IDs belong to the confirmed account.
 */

import { create } from "zustand";
import { ownerPersistStorage, removeOwnerPersistedState } from "@/lib/owner-persist-storage";
import { safePersistStorage } from "@/lib/safe-persist-storage";
import { createJSONStorage, persist } from "zustand/middleware";
import { getAssistantIdentityUserId } from "@/lib/assistant/identity";

/** The preview column's share of the content area, in percent. */
export const MACHINE_PANE_PERCENT = { min: 25, max: 75, default: 50 } as const;

/** Earlier builds saved a preset name; keep the user's choice. */
const LEGACY_PRESETS: Readonly<Record<string, number>> = { narrow: 35, half: 50, wide: 65 };

/** Hidden threads are remembered up to this many, newest kept. */
const HIDDEN_LIMIT = 50;

export function clampMachinePanePercent(value: unknown): number {
  const number = typeof value === "number" ? value : typeof value === "string" ? LEGACY_PRESETS[value] : undefined;
  if (number === undefined || !Number.isFinite(number)) return MACHINE_PANE_PERCENT.default;
  return Math.round(Math.min(MACHINE_PANE_PERCENT.max, Math.max(MACHINE_PANE_PERCENT.min, number)));
}

interface MachinePaneState {
  readonly ownerUserId: string | null;
  readonly bindAccount: (userId: string | null) => void;
  readonly percent: number;
  readonly hidden: readonly string[];
  readonly setPercent: (percent: number) => void;
  readonly setHidden: (conversationId: string, hidden: boolean) => void;
}

const hiddenStorage = ownerPersistStorage();
const machineStorage = {
  getItem(name: string) {
    const owned = hiddenStorage.getItem(name);
    const shared = safePersistStorage.getItem(name);
    if (!owned) return shared;
    try {
      const value = JSON.parse(owned);
      const width = JSON.parse(shared ?? "null")?.state?.percent;
      if (width !== undefined) value.state.percent = width;
      return JSON.stringify(value);
    } catch { return shared; }
  },
  setItem(name: string, value: string) {
    hiddenStorage.setItem(name, value);
    const parsed = JSON.parse(value);
    safePersistStorage.setItem(name, JSON.stringify({ version: 1, state: { percent: parsed.state.percent } }));
  },
  removeItem: hiddenStorage.removeItem,
};

export const useMachinePaneStore = create<MachinePaneState>()(
  persist(
    (set, get) => ({
      ownerUserId: null,
      bindAccount: (userId) => {
        if (get().ownerUserId !== userId) {
          const previous = get().ownerUserId;
          set({ ownerUserId: null, hidden: [] });
          removeOwnerPersistedState("nyxid.machine-pane", previous);
        }
        void useMachinePaneStore.persist.rehydrate();
        if (get().ownerUserId !== userId) set({ ownerUserId: userId, hidden: [] });
      },
      percent: MACHINE_PANE_PERCENT.default,
      hidden: [],
      setPercent: (percent) => set({ percent: clampMachinePanePercent(percent) }),
      setHidden: (conversationId, hidden) =>
        set((state) => {
          const rest = state.hidden.filter((id) => id !== conversationId);
          return { ownerUserId: getAssistantIdentityUserId(), hidden: hidden ? [...rest, conversationId].slice(-HIDDEN_LIMIT) : rest };
        }),
    }),
    {
      storage: createJSONStorage(() => machineStorage),
      name: "nyxid.machine-pane",
      version: 1,
      partialize: (state) => ({ percent: state.percent, ownerUserId: state.ownerUserId, hidden: state.hidden }),
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        return {
          ...current,
          percent: clampMachinePanePercent(p.percent ?? p.width),
          ownerUserId: typeof p.ownerUserId === "string" ? p.ownerUserId : null,
          hidden: typeof p.ownerUserId === "string" && (!getAssistantIdentityUserId() || p.ownerUserId === getAssistantIdentityUserId()) && Array.isArray(p.hidden)
            ? p.hidden.filter((id): id is string => typeof id === "string").slice(-HIDDEN_LIMIT) : [],
        };
      },
    },
  ),
);
