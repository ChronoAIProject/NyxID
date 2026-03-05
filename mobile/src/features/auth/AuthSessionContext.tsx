import { createContext, PropsWithChildren, useContext, useEffect, useMemo, useState } from "react";
import {
  clearStoredAuthSession,
  loadStoredAuthSession,
  persistAuthSession,
  StoredAuthSession,
} from "../../lib/auth/sessionStore";
import {
  activatePushAfterLogin,
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
      .finally(() => {
        if (!active) return;
        setIsRestoring(false);
      });

    return () => {
      active = false;
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
      await deactivatePushOnLogout();
      await clearLocalPushRegistrationState();
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
