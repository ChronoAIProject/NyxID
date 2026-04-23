/**
 * Telemetry consent — client-side policy state.
 *
 * Persisted to localStorage so the user's choice survives reloads. The
 * value is policy (not secret), mirroring the mobile consent store
 * (`mobile/src/lib/consent.ts`) for cross-surface parity. See
 * `docs/TELEMETRY.md` §8 gates.
 */

import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export interface ConsentState {
  /** True iff the user has opted in to telemetry on this machine. */
  enabled: boolean;
  /** True iff the user has answered the consent prompt at least once.
   * Used to decide whether to render the banner on next app boot. */
  asked: boolean;
  /** Persist a consent decision (called by the banner and the
   * Settings-page toggle). Sets `asked = true` either way. */
  setConsent(enabled: boolean): void;
  /** Used once when a user reverses their choice from Settings. */
  clearConsent(): void;
}

export const useConsentStore = create<ConsentState>()(
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
    }
  )
);
