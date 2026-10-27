/**
 * The machine column beside a chat: its share of the content area (one for
 * every chat in this browser, set by dragging the divider) and the threads
 * where the user hid it. Persisted per browser and validated on load, like
 * the display preferences.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

/** The machine column's share of the content area, in percent. */
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
  readonly percent: number;
  readonly hidden: readonly string[];
  readonly setPercent: (percent: number) => void;
  readonly setHidden: (conversationId: string, hidden: boolean) => void;
}

export const useMachinePaneStore = create<MachinePaneState>()(
  persist(
    (set) => ({
      percent: MACHINE_PANE_PERCENT.default,
      hidden: [],
      setPercent: (percent) => set({ percent: clampMachinePanePercent(percent) }),
      setHidden: (conversationId, hidden) =>
        set((state) => {
          const rest = state.hidden.filter((id) => id !== conversationId);
          return { hidden: hidden ? [...rest, conversationId].slice(-HIDDEN_LIMIT) : rest };
        }),
    }),
    {
      name: "nyxid.machine-pane",
      version: 1,
      partialize: (state) => ({ percent: state.percent, hidden: state.hidden }),
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        return {
          ...current,
          percent: clampMachinePanePercent(p.percent ?? p.width),
          hidden: Array.isArray(p.hidden)
            ? p.hidden.filter((id): id is string => typeof id === "string").slice(-HIDDEN_LIMIT)
            : current.hidden,
        };
      },
    },
  ),
);
