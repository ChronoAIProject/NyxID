import { crashLifecycle } from "../../lib/crashReporting/runtime";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { createContext, PropsWithChildren, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import { advanceAuthEpoch } from "../../lib/auth/authEpoch";
import { getAuthAccountId, setAuthAccountId } from "../../lib/auth/authAccount";
import {
  clearStoredAuthSession,
  authSessionGeneration,
  loadStoredAuthSession,
  peekStoredAuthSession,
  persistAuthSession,
  StoredAuthSession,
} from "../../lib/auth/sessionStore";
import {
  activatePushAfterLogin,
  deactivatePushOnLogout,
  capturePushLogout,
} from "../../lib/notifications/pushNotifications";
import { clearPendingApprovalRefreshSignal } from "../../lib/notifications/approvalRefreshSignal";
import { getCurrentUserProfileRequest, refreshAccessTokenIfNeeded, setSessionInvalidationListener } from "../../lib/api/http";
import { isEmailAllowed, ALLOWED_EMAILS } from "../../lib/env";
import {
  identify as telemetryIdentify,
  initTelemetry,
  readExpoTelemetryConfig,
  reset as telemetryReset,
  shutdownTelemetry,
} from "../../lib/telemetry";
import { decodeJwtSub } from "../../lib/auth/jwt";
import { useMobileConsent } from "../../lib/consent";

const PROACTIVE_REFRESH_INTERVAL_MS = 10 * 60 * 1000;
const RESTORE_TIMEOUT_MS = 6000;
const SIGN_OUT_PUSH_CAPTURE_TIMEOUT_MS = 250;
async function captureSignOutPush(accessToken?: string): Promise<Awaited<ReturnType<typeof capturePushLogout>>> {
  const fallback = { accessToken, registration: null };
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      capturePushLogout(accessToken),
      new Promise<typeof fallback>(resolve => { timeout = setTimeout(() => resolve(fallback), SIGN_OUT_PUSH_CAPTURE_TIMEOUT_MS); }),
    ]);
  } catch (error) {
    if (__DEV__) console.warn("[push] sign-out capture unavailable; cleanup will retry", error);
    return fallback;
  } finally {
    clearTimeout(timeout);
  }
}
const RESTORE_RETRY_DELAY_MS = 500;
const RESTORE_MAX_RETRY_DELAY_MS = 30_000;
export const SESSION_STORAGE_FAILED_MESSAGE =
  "Signed in, but this device couldn't save your session. Please try again.";

type AuthSessionContextValue = {
  isAuthenticated: boolean;
  isRestoring: boolean;
  signInWithSession: (session: StoredAuthSession) => Promise<void>;
  signOut: () => Promise<void>;
};

const AuthSessionContext = createContext<AuthSessionContextValue | null>(null);

export function AuthSessionProvider({ children }: PropsWithChildren) {
  const { t } = useAppLanguage();
  const queryClient = useQueryClient();
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [isRestoring, setIsRestoring] = useState(true);
  useEffect(() => { void crashLifecycle?.setRestoring(isRestoring); }, [isRestoring]);
  const admittedSessionRef = useRef<StoredAuthSession | null>(null);
  const signOutFlight = useRef<Promise<void> | null>(null);
  const authIntentRef = useRef(0);

  const performSignOut = useCallback((): Promise<void> => {
    if (signOutFlight.current) return signOutFlight.current;
    authIntentRef.current++;
    const expectedGeneration = authSessionGeneration();
    const run = (async () => {
      // Capture credentials/registration before clearing; no queue or network
      // teardown may delay the authoritative local sign-out.
      const session = peekStoredAuthSession() ?? admittedSessionRef.current;
      const push = await captureSignOutPush(session?.accessToken);
      // Rotation may finish during the bounded registration read. Use the
      // latest coordinator-owned pair, not the restore/sign-in snapshot.
      push.accessToken = (peekStoredAuthSession() ?? session)?.accessToken;
      if (authSessionGeneration() !== expectedGeneration)
        throw Object.assign(new Error("auth_session_changed"), { name: "AbortError" });
      await clearStoredAuthSession();
      if (authSessionGeneration() !== expectedGeneration + 1)
        throw Object.assign(new Error("auth_session_changed"), { name: "AbortError" });
      crashLifecycle?.disarm();
      admittedSessionRef.current = null;
      setAuthAccountId(null);
      advanceAuthEpoch();
      clearPendingApprovalRefreshSignal();
      telemetryReset();
      queryClient.clear();
      setIsAuthenticated(false);
      setIsRestoring(false);
      // The durable tombstone owns success. Pending-unbind recovery and native
      // cleanup are independent and use the captured, old account credentials.
      void deactivatePushOnLogout(push).catch((error) => {
        if (__DEV__) console.warn("[push] post-sign-out cleanup failed", error);
      });
    })();
    const flight = run.finally(() => { if (signOutFlight.current === flight) signOutFlight.current = null; });
    signOutFlight.current = flight;
    return flight;
  }, [queryClient]);

  // Register the HTTP-layer session invalidation hook so that a 401
  // after failed refresh triggers a full sign-out (React state + storage
  // + push cleanup) instead of silently clearing SecureStore.
  useEffect(() => {
    setSessionInvalidationListener(() => {
      void performSignOut().catch((error) => {
        if (__DEV__) console.warn("[auth] durable sign-out failed; retry required", error);
      });
    });
    return () => setSessionInvalidationListener(null);
  }, [performSignOut]);

  useEffect(() => {
    if (!isAuthenticated) return;

    let active = true;
    const checkRefresh = () => {
      // iOS suspends a backgrounded app within seconds. A refresh the server
      // completes after suspension rotates the refresh token without the app
      // ever storing the successor, so only start refreshes in the foreground.
      if (!active || AppState.currentState !== "active") return;
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

  const { enabled: consentEnabled, accountId, isHydrated: consentHydrated } = useMobileConsent();
  useEffect(() => {
    if (isRestoring || !isAuthenticated || !consentHydrated || !consentEnabled || !accountId) {
      shutdownTelemetry(); return;
    }
    void initTelemetry({ ...readExpoTelemetryConfig(), consent: true });
    return shutdownTelemetry;
  }, [isRestoring, isAuthenticated, consentHydrated, consentEnabled, accountId]);

  useEffect(() => {
    let active = true;
    let settled = false;
    // Sign-in or sign-out during a slow or retried restore owns the state.
    const restoreGeneration = authSessionGeneration();
    const restoreIntent = authIntentRef.current;
    const superseded = () => authSessionGeneration() !== restoreGeneration || authIntentRef.current !== restoreIntent;
    let inFlight = false;
    let foregroundFailures = 0;
    let restoreTimeout: ReturnType<typeof setTimeout> | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    const armTimeout = () => {
      clearTimeout(restoreTimeout);
      restoreTimeout = setTimeout(() => {
      if (!active || settled) return;
        // A background launch stays on the splash: the keychain may only be
        // waiting for the device to unlock. The timeout re-arms on foreground.
        if (AppState.currentState !== "active") return;
      if (__DEV__) console.warn("[auth] restore session timeout, continuing without cache");
      setIsRestoring(false);
    }, RESTORE_TIMEOUT_MS);
    };
    const settle = () => {
      settled = true;
      clearTimeout(restoreTimeout);
      clearTimeout(retryTimer);
      foregroundSubscription.remove();
      setIsRestoring(false);
    };
    const foregroundSubscription = AppState.addEventListener(
      "change",
      (next) => {
        if (next !== "active" || !active || settled) return;
        armTimeout();
        clearTimeout(retryTimer);
        restore();
      },
    );

    const restore = () => {
      if (!active || settled || inFlight) return;
      if (superseded()) return settle();
      inFlight = true;
      void loadStoredAuthSession()
      .then((session) => {
        if (!active || settled) return;
          if (superseded()) return settle();
        admittedSessionRef.current = session;
        setIsAuthenticated(Boolean(session));
        setAuthAccountId(
          session ? (decodeJwtSub(session.accessToken) ?? session.userId ?? null)
              : null,
        );
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
          settle();
        })
      .catch((error) => {
        if (__DEV__) console.warn("[auth] restore session failed", error);
        if (!active || settled) return;
          if (superseded()) return settle();
          // An unreadable keychain never proves sign-out. Release the splash
          // after the timeout, but retain foreground and bounded active retries.
          if (AppState.currentState !== "active") return;
          const delay = Math.min(RESTORE_RETRY_DELAY_MS * 2 ** Math.min(foregroundFailures++, 6), RESTORE_MAX_RETRY_DELAY_MS);
          retryTimer = setTimeout(() => { if (AppState.currentState === "active") restore(); }, delay);
        }).finally(() => { inFlight = false; });
    };

    armTimeout();
    restore();

    return () => {
      active = false;
      clearTimeout(restoreTimeout);
      clearTimeout(retryTimer);
      foregroundSubscription.remove();
    };
  }, []);

  const value = useMemo<AuthSessionContextValue>(() => {
    const signInWithSession = async (session: StoredAuthSession) => {
      authIntentRef.current++;
      const nextAccountId = decodeJwtSub(session.accessToken) ?? session.userId ?? null;
      if (getAuthAccountId() !== nextAccountId) setAuthAccountId(null);
      advanceAuthEpoch();
      await queryClient.cancelQueries();
      queryClient.clear();
      try {
        await persistAuthSession(session);
      } catch (error) {
        // A newer sign-in or sign-out owns the session; nothing to report.
        if (error instanceof Error && error.message === "auth_session_changed")
          throw error;
        if (__DEV__) console.warn("[auth] could not store the session", error);
        throw new Error(t(SESSION_STORAGE_FAILED_MESSAGE));
      }
      const expectedGeneration = authSessionGeneration();

      // Gate: if an allowlist is configured, verify the user's email before proceeding
      if (ALLOWED_EMAILS.length > 0) {
        let profile: Awaited<ReturnType<typeof getCurrentUserProfileRequest>>;
        try {
          profile = await getCurrentUserProfileRequest();
        } catch {
          if (authSessionGeneration() === expectedGeneration) await clearStoredAuthSession();
          throw new Error(t("Unable to verify account access. Please try again."));
        }
        // A policy denial is a successful verification. Keep it outside the
        // request catch so localized copy never participates in control flow.
        if (!isEmailAllowed(profile.email)) {
          if (authSessionGeneration() === expectedGeneration) await clearStoredAuthSession();
          throw new Error(t("Access restricted. Your account is not authorized for this app."));
        }
      }

      if (authSessionGeneration() !== expectedGeneration) throw new Error("auth_session_changed");
      admittedSessionRef.current = session;
      setAuthAccountId(nextAccountId);
      setIsAuthenticated(true);
      setIsRestoring(false);
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
  }, [isAuthenticated, isRestoring, performSignOut, queryClient, t]);

  return (
    <AuthSessionContext.Provider value={value}>{children}</AuthSessionContext.Provider>
  );
}

export function useAuthSession() {
  useAppLanguage();
  const context = useContext(AuthSessionContext);
  if (!context) {
    throw new Error("useAuthSession must be used within AuthSessionProvider");
  }
  return context;
}
