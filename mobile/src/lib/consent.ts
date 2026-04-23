/**
 * Mobile telemetry consent store.
 *
 * Mirrors `frontend/src/stores/consent-store.ts` so the cross-surface
 * behavior is identical: a single `{ enabled, asked }` policy flag
 * persisted across app restarts. Backed by AsyncStorage (consent is
 * policy, not secret; SecureStore would be overkill for one boolean —
 * see `docs/TELEMETRY.md` §5.3).
 *
 * The store is consumed by `app/App.tsx` (gating telemetry init) and
 * by the Settings screen toggle.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';

export interface MobileConsentState {
  enabled: boolean;
  asked: boolean;
  setConsent(enabled: boolean): void;
  clearConsent(): void;
}

export const useMobileConsent = create<MobileConsentState>()(
  persist(
    (set) => ({
      enabled: false,
      asked: false,
      setConsent(enabled) {
        set({ enabled, asked: true });
      },
      clearConsent() {
        set({ enabled: false, asked: false });
      },
    }),
    {
      name: 'nyxid.telemetry_consent',
      version: 1,
      storage: createJSONStorage(() => AsyncStorage),
    }
  )
);
