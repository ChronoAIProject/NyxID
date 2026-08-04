import { create } from "zustand";

export interface OAuthPopupAttempt {
  readonly launchId: string;
  readonly nonce: string | null;
  readonly keyId: string | null;
  readonly slug: string;
  readonly startedAt: number;
}

interface OAuthPopupState {
  readonly attempt: OAuthPopupAttempt | null;
  begin(attempt: OAuthPopupAttempt): boolean;
  setNonce(launchId: string, nonce: string): void;
  setKeyId(launchId: string, keyId: string): void;
  end(launchId: string): void;
}

export const useOAuthPopupStore = create<OAuthPopupState>((set, get) => ({
  attempt: null,
  begin: (attempt) => {
    if (get().attempt !== null) return false;
    set({ attempt });
    return true;
  },
  setNonce: (launchId, nonce) => {
    const current = get().attempt;
    if (current?.launchId === launchId) {
      set({ attempt: { ...current, nonce } });
    }
  },
  setKeyId: (launchId, keyId) => {
    const current = get().attempt;
    if (current?.launchId === launchId) {
      set({ attempt: { ...current, keyId } });
    }
  },
  end: (launchId) => {
    if (get().attempt?.launchId === launchId) set({ attempt: null });
  },
}));
