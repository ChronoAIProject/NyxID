import { useCallback, useEffect, useRef, type RefObject } from "react";
import {
  getActionFromState,
  getStateFromPath,
  type NavigationContainerRefWithCurrent,
} from "@react-navigation/native";
import * as Notifications from "expo-notifications";
import type { RootStackParamList } from "./navigationTypes";
import { navigationLinkingConfig } from "./navigationLinking";
import { workspaceStateFromPath } from "./workspaceCanonicalLinks";
import { challengeNavigationAction } from "./navigationHierarchy";
import {
  extractUrlFromNotificationResponse,
  notificationLinkConfig,
  reportDeepLinkOpened,
  subscribeDirectLinkOpened,
} from "./linking";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../lib/auth/authAccount";
import { nativeNotificationUrl } from "../lib/notifications/notificationDestination";
import { notificationPushSignalFromResponse } from "../lib/notifications/notificationPushSignal";

type AccountBinding = { accountId: string | null; invalidated: boolean };

function bindAccount(binding: AccountBinding | null, accountId: string | null) {
  if (!binding || binding.invalidated) return;
  if (binding.accountId === null) binding.accountId = accountId;
  else if (binding.accountId !== accountId) binding.invalidated = true;
}

/** One queue for cold, background and foreground taps; the latest tap owns the destination. */
export function useNotificationNavigation(
  navigation: NavigationContainerRefWithCurrent<RootStackParamList>,
  guards: RefObject<Set<symbol>>,
  onNotificationNavigated?: (notificationId: string) => void | Promise<void>,
) {
  const pending = useRef<
    (AccountBinding & { url: string; notificationId: string | null }) | null
  >(null);
  const coldBinding = useRef<AccountBinding | null>(null);
  const handled = useRef<string | undefined>(undefined);
  const intentRevision = useRef(0);
  const flush = useCallback(() => {
    const accountId = getAuthAccountId();
    // Publish account changes synchronously so batched sign-out/sign-in cannot
    // revive an old tap or rebind an anonymous delayed read a second time.
    bindAccount(pending.current, accountId);
    bindAccount(coldBinding.current, accountId);
    if (pending.current?.invalidated) pending.current = null;
    if (
      !pending.current ||
      !accountId ||
      !navigation.isReady() ||
      guards.current.size
    )
      return;
    const root = navigation.getRootState();
    if (!root.routes.some((route) => route.name === "Workspace")) return;
    const destinationAction = () => {
      try {
        const parsed = new URL(pending.current!.url);
        const challengeId = parsed.searchParams.get("challengeId");
        if (challengeId) return challengeNavigationAction(root, challengeId);
        const state = workspaceStateFromPath(
          `${parsed.host}${parsed.pathname}${parsed.search}`,
          (path) => getStateFromPath(path, navigationLinkingConfig),
        );
        return state
          ? getActionFromState(state, navigationLinkingConfig)
          : undefined;
      } catch {
        return undefined;
      }
    };
    const next = pending.current;
    let action = destinationAction();
    const supportedDestination = Boolean(action);
    if (!action) {
      const fallback = getStateFromPath("activity", navigationLinkingConfig);
      action =
        fallback && getActionFromState(fallback, navigationLinkingConfig);
    }
    if (action) {
      pending.current = null;
      navigation.dispatch(action);
      // Fallback Activity is useful for unknown pushes, but does not mean
      // the addressed notification was opened. Never mark that row read.
      if (
        supportedDestination &&
        next.notificationId &&
        getAuthAccountId() === next.accountId
      )
        void onNotificationNavigated?.(next.notificationId);
    }
  }, [navigation, guards, onNotificationNavigated]);
  const open = useCallback(
    (url: string, notificationId: string | null = null) => {
      const normalized = nativeNotificationUrl(url, notificationLinkConfig);
      if (!normalized) return;
      intentRevision.current++;
      pending.current = {
        url: normalized,
        notificationId,
        accountId: getAuthAccountId(),
        invalidated: false,
      };
      reportDeepLinkOpened(normalized);
      flush();
    },
    [flush],
  );
  useEffect(() => {
    const unsubscribe = subscribeAuthAccount(flush);
    flush();
    return unsubscribe;
  }, [flush]);

  useEffect(
    () =>
      subscribeDirectLinkOpened(() => {
        intentRevision.current++;
        pending.current = null;
      }),
    [],
  );

  useEffect(() => {
    let mounted = true;
    let receivedTap = false;
    const coldRevision = intentRevision.current;
    const binding: AccountBinding = {
      accountId: getAuthAccountId(),
      invalidated: false,
    };
    coldBinding.current = binding;
    const handle = (response: Notifications.NotificationResponse | null) => {
      if (!mounted || !response) return;
      const id = `${response.notification.request.identifier}:${response.actionIdentifier}`;
      if (handled.current === id) return;
      const destination = extractUrlFromNotificationResponse(response);
      handled.current = id;
      open(
        destination ?? `${notificationLinkConfig.scheme}://activity`,
        destination
          ? notificationPushSignalFromResponse(response)?.notificationId
          : null,
      );
      void Notifications.clearLastNotificationResponseAsync().catch(() => {});
    };
    const subscription = Notifications.addNotificationResponseReceivedListener(
      (response) => {
        receivedTap = true;
        handle(response);
      },
    );
    void Notifications.getLastNotificationResponseAsync()
      .then((response) => {
        if (!mounted || receivedTap || intentRevision.current !== coldRevision)
          return;
        if (binding.invalidated) {
          if (response)
            void Notifications.clearLastNotificationResponseAsync().catch(
              () => {},
            );
          return;
        }
        handle(response);
      })
      .catch(() => {});
    return () => {
      mounted = false;
      subscription.remove();
    };
  }, [open]);
  return { open, flush };
}
