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
import { getCurrentUserProfileRequest, refreshAccessTokenIfNeeded, setSessionInvalidationListener } from "../../lib/api/http";
import { isEmailAllowed, ALLOWED_EMAILS } from "../../lib/env";
import {
  identify as telemetryIdentify,
  initTelemetry,
  readExpoTelemetryConfig,
  reset as telemetryReset,
} from "../../lib/telemetry";
import { decodeJwtSub } from "../../lib/auth/jwt";
import { useMobileConsent } from "../../lib/consent";

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
      // Clear telemetry identity before we wipe auth state so the
      // very next event carries a fresh anon distinct_id rather than
      // the ex-user's id. Safe no-op when telemetry is off.
      telemetryReset();
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

  // Initialize telemetry once the session has finished restoring AND
  // the user has opted in. Idempotent: `initTelemetry` guards against
  // double-invoke internally so effect re-runs are safe.
  const { enabled: consentEnabled } = useMobileConsent();
  useEffect(() => {
    if (isRestoring) return;
    if (!consentEnabled) return;
    const cfg = readExpoTelemetryConfig();
    void initTelemetry({
      dsn: cfg.dsn,
      host: cfg.host,
      shareBack: cfg.shareBack,
      consent: true,
    });
  }, [isRestoring, consentEnabled]);

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
          // If we restored an authenticated session with a known
          // `userId` (persisted or JWT-derived by `loadStoredAuthSession`),
          // identify to telemetry immediately so post-boot events
          // attribute to the user rather than the anon id. Safe no-op
          // when telemetry is off or consent not granted.
          if (session.userId) {
            telemetryIdentify(session.userId);
          }
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

      // Gate: if an allowlist is configured, verify the user's email before proceeding
      if (ALLOWED_EMAILS.length > 0) {
        try {
          const profile = await getCurrentUserProfileRequest();
          if (!isEmailAllowed(profile.email)) {
            await clearStoredAuthSession();
            throw new Error("Access restricted. Your account is not authorized for this app.");
          }
        } catch (error) {
          // Re-throw allowlist rejections; swallow profile-fetch failures only
          if (error instanceof Error && error.message.includes("Access restricted")) {
            throw error;
          }
          await clearStoredAuthSession();
          throw new Error("Unable to verify account access. Please try again.");
        }
      }

      setIsAuthenticated(true);
      // Identify to telemetry as soon as the session is persisted.
      // `persistAuthSession` derives/persists userId internally but does
      // not mutate the passed object, so we also derive it here from
      // the access token's JWT `sub` claim. Falls back to the caller-
      // supplied value when present.
      const userId = session.userId ?? decodeJwtSub(session.accessToken);
      if (userId) {
        telemetryIdentify(userId);
      }
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
