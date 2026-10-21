/** Account-bound local device choices, persisted separately from server consent. */
import AsyncStorage from '@react-native-async-storage/async-storage';
import { createContext, useCallback, useContext, useEffect, useMemo, useSyncExternalStore, type PropsWithChildren } from 'react';
import { analyticsConsentState, type ConsentSnapshot } from './analyticsConsentState';
import { createConsentPersistence } from './consentPersistence';
const STORAGE_KEY = 'nyxid.telemetry_consent';
export type MobileConsentContextValue = ConsentSnapshot & { setConsent(enabled: boolean): void; clearConsent(): void; retryPersistence(): void };
const MobileConsentContext = createContext<MobileConsentContextValue | null>(null);
export function MobileConsentProvider({ children }: PropsWithChildren) {
  const state = useSyncExternalStore(analyticsConsentState.subscribe, analyticsConsentState.getSnapshot);
  useEffect(() => {
    let cancelled = false;
    void AsyncStorage.getItem(STORAGE_KEY).catch(() => null).then((raw) => {
      if (!cancelled) analyticsConsentState.hydrate(raw);
    });
    return () => { cancelled = true; };
  }, []);
  const setConsent = useCallback((enabled: boolean) => { analyticsConsentState.setConsent(enabled); void persist(); }, []);
  const clearConsent = useCallback(() => { analyticsConsentState.clearConsent(); void persist(); }, []);
  const value = useMemo(() => ({ ...state, setConsent, clearConsent, retryPersistence: () => { void persist(); } }), [state, setConsent, clearConsent]);
  return <MobileConsentContext.Provider value={value}>{children}</MobileConsentContext.Provider>;
}
const persist = createConsentPersistence(analyticsConsentState, (serialized) => AsyncStorage.setItem(STORAGE_KEY, serialized));
export function useMobileConsent(): MobileConsentContextValue {
  const context = useContext(MobileConsentContext);
  if (!context) throw new Error('MobileConsentProvider required');
  return context;
}
