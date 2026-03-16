import { createContext, PropsWithChildren, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
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
import { refreshAccessTokenIfNeeded, setSessionInvalidationListener } from "../../lib/api/http";

const PROACTIVE_REFRESH_INTERVAL_MS = 10 * 60 * 1000;

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
  const isSigningOutRef = useRef(false);

  const performSignOut = useCallback(async () => {
    if (isSigningOutRef.current) return;
    isSigningOutRef.current = true;
    try {
      const pushUnlinked = await deactivatePushOnLogout();
      if (pushUnlinked) {
        await clearLocalPushRegistrationState();
      } else {
        await clearPendingPushSyncSignal();
      }
      await clearStoredAuthSession();
      setIsAuthenticated(false);
    } finally {
      isSigningOutRef.current = false;
    }
  }, []);

  // Register the HTTP-layer session invalidation hook so that a 401
  // after failed refresh triggers a full sign-out (React state + storage
  // + push cleanup) instead of silently clearing SecureStore.
  useEffect(() => {
    setSessionInvalidationListener(() => {
      void performSignOut();
    });
    return () => setSessionInvalidationListener(null);
  }, [performSignOut]);

  useEffect(() => {
    if (!isAuthenticated) return;

    let active = true;
    const checkRefresh = () => {
      if (!active) return;
      void refreshAccessTokenIfNeeded().catch((error) => {
        if (__DEV__) console.warn("[auth] proactive refresh check failed", error);
      });
    };

    checkRefresh();
    const interval = setInterval(checkRefresh, PROACTIVE_REFRESH_INTERVAL_MS);
    const appStateSubscription = AppState.addEventListener("change", (nextState) => {
      if (nextState === "active") {
        checkRefresh();
      }
    });

    return () => {
      active = false;
      clearInterval(interval);
      appStateSubscription.remove();
    };
  }, [isAuthenticated]);

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

    return {
      isAuthenticated,
      isRestoring,
      signInWithSession,
      signOut: performSignOut,
    };
  }, [isAuthenticated, isRestoring, performSignOut]);

  return <AuthSessionContext.Provider value={value}>{children}</AuthSessionContext.Provider>;
}

export function useAuthSession() {
  const context = useContext(AuthSessionContext);
  if (!context) {
    throw new Error("useAuthSession must be used within AuthSessionProvider");
  }
  return context;
}
