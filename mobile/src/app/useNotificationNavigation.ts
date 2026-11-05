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
} from "./linking";
import { getAuthEpoch } from "../lib/auth/authEpoch";
import { nativeNotificationUrl } from "../lib/notifications/notificationDestination";

/** One queue for cold, background and foreground taps; the latest tap owns the destination. */
export function useNotificationNavigation(
  navigation: NavigationContainerRefWithCurrent<RootStackParamList>,
  guards: RefObject<Set<symbol>>,
) {
  const pending = useRef<{ url: string; epoch: number } | null>(null);
  const handled = useRef<string | undefined>(undefined);
  const intentRevision = useRef(0);
  const flush = useCallback(() => {
    if (pending.current?.epoch !== getAuthEpoch()) pending.current = null;
    if (!pending.current || !navigation.isReady() || guards.current.size)
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
    let action = destinationAction();
    if (!action) {
      const fallback = getStateFromPath("activity", navigationLinkingConfig);
      action =
        fallback && getActionFromState(fallback, navigationLinkingConfig);
    }
    if (action) {
      pending.current = null;
      navigation.dispatch(action);
    }
  }, [navigation, guards]);
  const open = useCallback(
    (url: string) => {
      const normalized = nativeNotificationUrl(url, notificationLinkConfig);
      if (!normalized) return;
      intentRevision.current++;
      pending.current = { url: normalized, epoch: getAuthEpoch() };
      reportDeepLinkOpened(normalized);
      flush();
    },
    [flush],
  );

  useEffect(() => {
    let mounted = true;
    let receivedTap = false;
    const coldRevision = intentRevision.current;
    const coldEpoch = getAuthEpoch();
    const handle = (response: Notifications.NotificationResponse | null) => {
      if (!mounted || !response) return;
      const id = `${response.notification.request.identifier}:${response.actionIdentifier}`;
      if (handled.current === id) return;
      const url =
        extractUrlFromNotificationResponse(response) ??
        `${notificationLinkConfig.scheme}://activity`;
      handled.current = id;
      open(url);
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
        if (
          !receivedTap &&
          intentRevision.current === coldRevision &&
          getAuthEpoch() === coldEpoch
        )
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
