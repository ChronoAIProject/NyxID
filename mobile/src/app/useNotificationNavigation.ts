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
} from "./linking";
import { nativeNotificationUrl } from "../lib/notifications/notificationDestination";

/** One queue for cold, background and foreground taps; the latest tap owns the destination. */
export function useNotificationNavigation(
  navigation: NavigationContainerRefWithCurrent<RootStackParamList>,
  guards: RefObject<Set<symbol>>,
) {
  const pending = useRef<string | null>(null);
  const handled = useRef<string | undefined>(undefined);
  const intentRevision = useRef(0);
  const flush = useCallback(() => {
    if (!pending.current || !navigation.isReady() || guards.current.size)
      return;
    const root = navigation.getRootState();
    if (!root.routes.some((route) => route.name === "Workspace")) return;
    const parsed = new URL(pending.current);
    const challengeId = parsed.searchParams.get("challengeId");
    const state =
      !challengeId &&
      workspaceStateFromPath(
        `${parsed.host}${parsed.pathname}${parsed.search}`,
        (path) => getStateFromPath(path, navigationLinkingConfig),
      );
    const action = challengeId
      ? challengeNavigationAction(root, challengeId)
      : state && getActionFromState(state, navigationLinkingConfig);
    pending.current = null;
    if (action) navigation.dispatch(action);
  }, [navigation, guards]);
  const open = useCallback(
    (url: string) => {
      const normalized = nativeNotificationUrl(url, notificationLinkConfig);
      if (!normalized) return;
      intentRevision.current++;
      pending.current = normalized;
      flush();
    },
    [flush],
  );

  useEffect(() => {
    let mounted = true;
    let receivedTap = false;
    const coldRevision = intentRevision.current;
    const handle = (response: Notifications.NotificationResponse | null) => {
      if (!mounted || !response) return;
      const id = `${response.notification.request.identifier}:${response.actionIdentifier}`;
      if (handled.current === id) return;
      const url = extractUrlFromNotificationResponse(response);
      if (!url) return;
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
        if (!receivedTap && intentRevision.current === coldRevision)
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
