import { createContext, PropsWithChildren, useContext, useEffect, useMemo, useState } from "react";
import {
  clearStoredAuthSession,
  loadStoredAuthSession,
  persistAuthSession,
  StoredAuthSession,
} from "../../lib/auth/sessionStore";
import {
  activatePushAfterLogin,
  clearPendingPushSyncSignal,
  clearLocalPushRegistrationState,
  deactivatePushOnLogout,
} from "../../lib/notifications/pushNotifications";

type AuthSessionContextValue = {
  isAuthenticated: boolean;
  isRestoring: boolean;
  signInWithSession: (session: StoredAuthSession) => Promise<void>;
  signOut: () => Promise<void>;
};

const AuthSessionContext = createContext<AuthSessionContextValue | null>(null);

export function AuthSessionProvider({ children }: PropsWithChildren) {
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [isRestoring, setIsRestoring] = useState(true);

  useEffect(() => {
    let active = true;
    const restoreTimeout = setTimeout(() => {
      if (!active) return;
      if (__DEV__) console.warn("[auth] restore session timeout, continuing without cache");
      setIsRestoring(false);
    }, 6000);

    void loadStoredAuthSession()
      .then((session) => {
        if (!active) return;
        setIsAuthenticated(Boolean(session));
        if (session) {
          void activatePushAfterLogin({ forceRegister: true })
            .then((result) => {
              if (__DEV__) {
                console.log("[push] activate after session restore", result);
              }
            })
            .catch((error) => {
              if (__DEV__) console.warn("[push] activate after session restore failed", error);
            });
        }
      })
      .catch((error) => {
        if (__DEV__) console.warn("[auth] restore session failed", error);
        if (!active) return;
        setIsAuthenticated(false);
      })
      .finally(() => {
        if (!active) return;
        clearTimeout(restoreTimeout);
        setIsRestoring(false);
      });

    return () => {
      active = false;
      clearTimeout(restoreTimeout);
    };
  }, []);

  const value = useMemo<AuthSessionContextValue>(() => {
    const signInWithSession = async (session: StoredAuthSession) => {
      await persistAuthSession(session);
      setIsAuthenticated(true);
      try {
        const pushResult = await activatePushAfterLogin({ forceRegister: true });
        if (__DEV__) {
          console.log("[push] activate after sign in", pushResult);
        }
      } catch (error) {
        if (__DEV__) console.warn("[push] activate after sign in failed", error);
      }
    };

    const signOut = async () => {
      const pushUnlinked = await deactivatePushOnLogout();
      if (pushUnlinked) {
        await clearLocalPushRegistrationState();
      } else {
        await clearPendingPushSyncSignal();
      }
      await clearStoredAuthSession();
      setIsAuthenticated(false);
    };

    return {
      isAuthenticated,
      isRestoring,
      signInWithSession,
      signOut,
    };
  }, [isAuthenticated, isRestoring]);

  return <AuthSessionContext.Provider value={value}>{children}</AuthSessionContext.Provider>;
}

export function useAuthSession() {
  const context = useContext(AuthSessionContext);
  if (!context) {
    throw new Error("useAuthSession must be used within AuthSessionProvider");
  }
  return context;
}
