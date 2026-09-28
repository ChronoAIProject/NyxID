import { create } from "zustand";
import {
  getAssistantIdentityUserId,
  subscribeAssistantIdentity,
} from "@/lib/assistant/identity";
import type { CreditsPayer } from "@/lib/credits-denial";

export interface CreditsDenial {
  readonly key: string;
  readonly payer: CreditsPayer;
  readonly actorId: string | null;
}

interface CreditsDenialState {
  readonly current: CreditsDenial | null;
  readonly seenKeys: ReadonlySet<string>;
  readonly notify: (denial: CreditsDenial) => void;
  readonly dismiss: () => void;
  readonly reset: () => void;
}

export const useCreditsDenialStore = create<CreditsDenialState>((set, get) => ({
  current: null,
  seenKeys: new Set(),
  notify: (denial) => {
    // Identity fence: the actor captured when the operation started must
    // still be the signed-in person, so a late failure never prompts another.
    if (!denial.actorId || denial.actorId !== getAssistantIdentityUserId()) {
      return;
    }
    const { current, seenKeys } = get();
    if (seenKeys.has(denial.key)) return;
    set({
      current: current ?? denial,
      seenKeys: new Set(seenKeys).add(denial.key),
    });
  },
  dismiss: () => set({ current: null }),
  reset: () => set({ current: null, seenKeys: new Set() }),
}));

// Logout and account switches cross this boundary (see `auth-store.ts`).
subscribeAssistantIdentity(() => useCreditsDenialStore.getState().reset());
