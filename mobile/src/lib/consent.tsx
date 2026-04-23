/**
 * Mobile telemetry consent — React Context + AsyncStorage.
 *
 * Matches mobile's existing Context-based state pattern (see
 * `features/auth/AuthSessionContext.tsx`). Stores two booleans
 * (`enabled`, `asked`) persisted across app restarts. Consent is
 * policy, not secret — AsyncStorage is the right fit; SecureStore
 * would be overkill for one flag.
 *
 * Consumed by `app/App.tsx` (gating telemetry init), by
 * `features/auth/AuthSessionContext.tsx` (propagating consent into
 * telemetry init), and by the Settings screen toggle.
 *
 * See `docs/TELEMETRY.md` §5.3 for the consent model.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from "react";

const STORAGE_KEY = "nyxid.telemetry_consent";

export interface MobileConsentState {
  enabled: boolean;
  asked: boolean;
}

export interface MobileConsentContextValue extends MobileConsentState {
  /** `true` once the AsyncStorage-backed hydration has completed. */
  isHydrated: boolean;
  setConsent(enabled: boolean): void;
  clearConsent(): void;
}

const MobileConsentContext = createContext<MobileConsentContextValue | null>(null);

export function MobileConsentProvider({ children }: PropsWithChildren) {
  const [state, setState] = useState<MobileConsentState>({
    enabled: false,
    asked: false,
  });
  const [isHydrated, setIsHydrated] = useState(false);

  useEffect(() => {
    let cancelled = false;
    AsyncStorage.getItem(STORAGE_KEY)
      .then((raw) => {
        if (cancelled || !raw) return;
        try {
          const parsed = JSON.parse(raw) as
            | Partial<MobileConsentState>
            | { state?: Partial<MobileConsentState> };
          // Tolerate the zustand `persist` middleware's historical shape
          // (`{ state: {...}, version: N }`) so preview installs from
          // before the Context migration keep their opt-in decision.
          const flags =
            parsed && typeof parsed === "object" && "state" in parsed && parsed.state
              ? parsed.state
              : (parsed as Partial<MobileConsentState>);
          setState({
            enabled: Boolean(flags?.enabled),
            asked: Boolean(flags?.asked),
          });
        } catch {
          // Corrupt payload — treat as first run.
        }
      })
      .finally(() => {
        if (!cancelled) setIsHydrated(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setConsent = useCallback((enabled: boolean) => {
    const next = { enabled, asked: true };
    setState(next);
    AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next)).catch(() => {
      // Persistence failure is non-fatal; in-memory state wins the session.
    });
  }, []);

  const clearConsent = useCallback(() => {
    const next = { enabled: false, asked: false };
    setState(next);
    AsyncStorage.removeItem(STORAGE_KEY).catch(() => {});
  }, []);

  const value = useMemo<MobileConsentContextValue>(
    () => ({ ...state, isHydrated, setConsent, clearConsent }),
    [state, isHydrated, setConsent, clearConsent],
  );

  return (
    <MobileConsentContext.Provider value={value}>
      {children}
    </MobileConsentContext.Provider>
  );
}

export function useMobileConsent(): MobileConsentContextValue {
  const ctx = useContext(MobileConsentContext);
  if (!ctx) {
    throw new Error(
      "useMobileConsent must be used within a <MobileConsentProvider>",
    );
  }
  return ctx;
}
