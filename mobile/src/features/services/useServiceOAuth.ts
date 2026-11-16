import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import * as WebBrowser from "expo-web-browser";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../../lib/auth/authAccount";
import { servicesApi } from "../../lib/api/servicesApi";
import { isApiError } from "../../lib/api/ApiError";
import { reconnectScopeOverride } from "./connectionPresentation";
import {
  validateBrowserUrl,
  type CatalogEntry,
  type ServiceConnection,
} from "./serviceContracts";
import {
  evaluateServiceOAuth,
  parseServiceOAuthReturn,
  serviceOAuthReturnUri,
  SERVICE_OAUTH_TTL_MS,
  type OAuthOutcome,
  type PendingServiceOAuth,
} from "./serviceOAuthModel";
import {
  clearServiceOAuth,
  loadServiceOAuth,
  recordServiceOAuthReturn,
  saveServiceOAuth,
  serviceOAuthReturnHint,
  serviceOAuthRevision,
  subscribeServiceOAuth,
} from "./serviceOAuthStore";

export type ServiceOAuthBanner = {
  attempt: PendingServiceOAuth;
  outcome: OAuthOutcome;
  storedAttempt?: PendingServiceOAuth;
};

export function useServiceOAuth(
  focused: boolean,
  changed: () => Promise<void>,
) {
  const [banner, setBanner] = useState<ServiceOAuthBanner>();
  const [checking, setChecking] = useState(false);
  const generation = useRef(0);
  const browserOpen = useRef(false);
  const checkLock = useRef(false);
  const queuedCheck = useRef(false);
  const latestCheck = useRef<(retry: boolean) => Promise<void>>(async () => {});
  const expiryTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );

  const check = useCallback(
    async (boundedRetry = false) => {
      if (!focused || !getAuthAccountId()) return;
      if (checkLock.current) {
        queuedCheck.current = true;
        return;
      }
      checkLock.current = true;
      setChecking(true);
      const epoch = generation.current;
      const accountId = getAuthAccountId();
      const revision = serviceOAuthRevision();
      const current = () =>
        epoch === generation.current &&
        accountId === getAuthAccountId() &&
        revision === serviceOAuthRevision();
      try {
        const attempt = await loadServiceOAuth();
        if (!attempt) {
          if (epoch === generation.current && accountId === getAuthAccountId())
            setBanner((previous) =>
              previous?.outcome === "pending" ||
              previous?.outcome === "unconfirmed" ||
              previous?.outcome === "success"
                ? undefined
                : previous,
            );
          return;
        }
        if (!current()) return;
        clearTimeout(expiryTimer.current);
        expiryTimer.current = setTimeout(
          () => {
            if (current()) {
              void clearServiceOAuth(attempt).catch(() => {});
              setBanner(undefined);
            }
          },
          Math.max(0, attempt.expiresAt - Date.now()),
        );
        let connection: ServiceConnection | undefined;
        try {
          connection = await servicesApi.connection(attempt.connectionId);
        } catch {
          /* Keep an uncertain attempt available for an explicit check. */
        }
        if (!current()) return;
        let outcome = evaluateServiceOAuth(
          attempt,
          connection,
          serviceOAuthReturnHint(),
        );
        if (
          boundedRetry &&
          (outcome === "pending" || outcome === "unconfirmed")
        ) {
          await new Promise<void>((resolve) => setTimeout(resolve, 500));
          if (!current()) return;
          try {
            connection = await servicesApi.connection(attempt.connectionId);
          } catch {
            /* The banner retains Check again. */
          }
          if (!current()) return;
          outcome = evaluateServiceOAuth(
            attempt,
            connection,
            serviceOAuthReturnHint(),
          );
        }
        const displayAttempt = connection?.label
          ? { ...attempt, catalogName: connection.label }
          : attempt;
        setBanner({
          attempt: displayAttempt,
          outcome,
          storedAttempt: attempt,
        });
        if (current()) await changed();
        if (current() && outcome !== "pending" && outcome !== "unconfirmed")
          await clearServiceOAuth(attempt);
      } finally {
        checkLock.current = false;
        if (epoch === generation.current) setChecking(false);
        if (queuedCheck.current) {
          queuedCheck.current = false;
          void latestCheck.current(true).catch(() => {});
        }
      }
    },
    [focused, changed],
  );
  latestCheck.current = check;

  useEffect(() => {
    generation.current++;
    if (!focused) return;
    void check().catch(() => {});
    const unsubscribe = subscribeServiceOAuth(() => {
      if (!browserOpen.current) void check(true).catch(() => {});
    });
    const accountSubscription = subscribeAuthAccount(() => {
      generation.current++;
      setBanner(undefined);
      setChecking(false);
    });
    const foreground = AppState.addEventListener("change", (state) => {
      if (state === "active" && !browserOpen.current)
        void check(true).catch(() => {});
    });
    return () => {
      generation.current++;
      clearTimeout(expiryTimer.current);
      unsubscribe();
      accountSubscription();
      foreground.remove();
    };
  }, [focused, check]);

  const start = async (
    connection: ServiceConnection,
    entry: CatalogEntry,
    repairing = false,
  ) => {
    const accountId = getAuthAccountId();
    if (!accountId) return;
    if (!entry.provider_config_id) {
      throw new Error("This service has no OAuth provider configured.");
    }
    const epoch = generation.current;
    const current = () =>
      epoch === generation.current && getAuthAccountId() === accountId;
    const startedAt = Date.now();
    const attempt: PendingServiceOAuth = {
      accountId,
      connectionId: connection.id,
      providerId: entry.provider_config_id,
      catalogSlug: entry.slug,
      // Keep the account's connection label in the banner. Catalog names can
      // be shared by several connections and are not what the user authorized.
      catalogName: connection.label,
      baselineStatus: connection.status,
      baselineAuthorizedAt: connection.last_authorized_at ?? null,
      startedAt,
      expiresAt: startedAt + SERVICE_OAUTH_TTL_MS,
    };
    await saveServiceOAuth(attempt);
    if (!current()) return;
    setBanner(undefined);
    browserOpen.current = true;
    let browserStarted = false;
    try {
      let response;
      const initiateArgs = [
        entry.provider_config_id,
        connection.id,
        connection.credential_source?.type === "org"
          ? connection.credential_source.org_id
          : undefined,
        repairing ? reconnectScopeOverride(connection, entry) : undefined,
      ] as const;
      try {
        response = await servicesApi.initiateOAuth(...initiateArgs);
      } catch (error) {
        if (!isApiError(error) || error.statusCode !== 400) throw error;
        response = await servicesApi.initiateOAuth(...initiateArgs, {
          appReturnUri: null,
        });
      }
      if (!current()) return;
      const authorizationUrl = validateBrowserUrl(response.authorization_url);
      browserStarted = true;
      const result = await WebBrowser.openAuthSessionAsync(
        authorizationUrl,
        serviceOAuthReturnUri(),
        { preferEphemeralSession: true },
      );
      if (!current()) return;
      if (result.type === "success") {
        const returnUri = serviceOAuthReturnUri();
        const hint = parseServiceOAuthReturn(
          result.url,
          returnUri.slice(0, returnUri.indexOf("://")),
        );
        if (hint) recordServiceOAuthReturn(hint);
      }
    } catch (error) {
      if (!browserStarted) {
        await clearServiceOAuth(attempt);
        if (current()) setBanner({ attempt, outcome: "failed" });
        throw error;
      }
      // A browser-launch error may follow a completed provider callback; retain
      // the attempt and read the server before deciding what happened.
      if (__DEV__)
        console.warn("[service-oauth] browser authorization unavailable");
    } finally {
      browserOpen.current = false;
      if (current()) await check(true);
    }
  };

  const retry = async () => {
    if (!banner || banner.attempt.accountId !== getAuthAccountId()) return;
    const { attempt } = banner;
    const connection = await servicesApi.connection(attempt.connectionId);
    const entry = await servicesApi.catalogEntry(attempt.catalogSlug);
    if (
      attempt.accountId === getAuthAccountId() &&
      entry.provider_config_id === attempt.providerId
    )
      await start(connection, entry, true);
  };
  const dismiss = async () => {
    const current = banner;
    setBanner(undefined);
    if (current)
      await clearServiceOAuth(current.storedAttempt ?? current.attempt).catch(
        () => {},
      );
  };
  return {
    banner,
    checking,
    start,
    retry,
    dismiss,
    check: () => check(true),
  };
}
