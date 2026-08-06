import { create } from "zustand";

export interface PendingConnectAuthorization {
  /** Placeholder key whose terminal status settles the card. */
  readonly keyId: string;
  /** Anchors both the poll cadence and the give-up deadline. */
  readonly startedAt: number;
}

interface PendingConnectState {
  readonly attempts: Readonly<Record<string, PendingConnectAuthorization>>;
  begin(blockId: string, attempt: PendingConnectAuthorization): void;
  end(blockId: string): void;
}

/**
 * Out-of-band authorizations an assistant action card is still waiting on,
 * keyed by block id.
 *
 * This lives outside React on purpose. The card's busy projection
 * (`status: "in_progress"`) is held in the transport's conversation mirror and
 * survives a history refetch — `applyHistoryResponse` deliberately preserves
 * local structured blocks because Aevatar's transcript is text-only. The
 * authorization that is supposed to *clear* that projection used to be
 * component state, so the two had different lifetimes: any remount (switching
 * conversations, navigating to a key page and back, a window-focus refetch
 * that re-keys message groups) destroyed the exit condition while the
 * disabling condition lived on. The card was then stuck at "Connecting" with
 * every control disabled and no writer left to move it.
 *
 * Module scope gives the watch the same lifetime as the projection it clears.
 * A full reload drops both together, which is consistent: the local block is
 * gone too, and the card comes back from the wire as `pending`.
 */
export const usePendingConnectStore = create<PendingConnectState>((set) => ({
  attempts: {},
  begin: (blockId, attempt) => {
    set((state) => ({ attempts: { ...state.attempts, [blockId]: attempt } }));
  },
  end: (blockId) => {
    set((state) => {
      if (!(blockId in state.attempts)) return state;
      return {
        attempts: Object.fromEntries(
          Object.entries(state.attempts).filter(([id]) => id !== blockId),
        ),
      };
    });
  },
}));
