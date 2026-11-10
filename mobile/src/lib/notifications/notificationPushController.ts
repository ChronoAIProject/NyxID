import type { QueryClient } from "@tanstack/react-query";
import { NOTIFICATIONS } from "../../features/notifications/notificationKeys";
import type { NotificationsApi } from "../api/notificationsTransport";
import {
  notificationPushSignalFromData,
  type NotificationPushSignal,
} from "./notificationPushSignal";

export type NotificationPushTap = Pick<
  NotificationPushSignal,
  "notificationId" | "deeplink"
>;

export type NotificationPushControllerOptions = {
  api: Pick<NotificationsApi, "markRead">;
  queryClient: Pick<QueryClient, "invalidateQueries">;
  /**
   * Dispatches a first-party path inside the app, exactly as a universal
   * link would resolve it. Returns false while the navigator cannot take it
   * (not ready, signed out, a critical flow in progress); the tap then waits
   * for the next `flush`.
   */
  openPath: (path: string) => boolean;
  isForeground: () => boolean;
  isOnline: () => boolean;
  /** Frontend origins whose links may select native UI. */
  frontendUrls: readonly string[];
  /** The app's own URL scheme (`nyxid://assistant?c=…`). */
  appScheme: string;
};

/**
 * The in-app path a push deeplink names, or null when it is not a
 * first-party link: another origin, credentials, a fragment or an unparsable
 * value never navigate and never mark anything read.
 */
export function notificationDeeplinkPath(
  deeplink: string | null | undefined,
  frontendUrls: readonly string[],
  appScheme: string,
): string | null {
  if (!deeplink) return null;
  let url: URL;
  try {
    url = new URL(deeplink);
  } catch {
    return null;
  }
  if (url.username || url.password || url.hash) return null;
  if (url.protocol === `${appScheme}:`) {
    if (!url.host) return null;
    return `/${url.host}${url.pathname}${url.search}`;
  }
  if (!/^https?:$/.test(url.protocol)) return null;
  for (const value of frontendUrls) {
    try {
      const trusted = new URL(value);
      if (/^https?:$/.test(trusted.protocol) && trusted.origin === url.origin)
        return `${url.pathname}${url.search}`;
    } catch {
      // An unset or malformed configured origin trusts nothing.
    }
  }
  return null;
}

/**
 * One surface for NyxBot notification pushes (SPEC D9): an arrival marks the
 * feed stale, and a tap, whether from the OS (background or cold start) or
 * the in-app toast's Open action, navigates to the push's canonical
 * destination first and marks the row read only once that navigation was
 * dispatched. A tap the navigator cannot take yet stays pending until
 * `flush` runs with the navigator ready and the owner signed in.
 */
export function createNotificationPushController(
  options: NotificationPushControllerOptions,
) {
  let pending: { path: string; notificationId: string | null } | null = null;
  const invalidate = (refetchType: "active" | "none") =>
    void options.queryClient.invalidateQueries({
      queryKey: NOTIFICATIONS,
      refetchType,
    });
  const markRead = async (notificationId: string | null) => {
    if (!notificationId) return;
    try {
      await options.api.markRead({ ids: [notificationId] });
    } catch {
      // The row stays unread; opening the thread reads it (useVisibleThreadRead).
    }
    invalidate("active");
  };
  const flush = (): Promise<void> | undefined => {
    if (!pending) return undefined;
    const next = pending;
    if (!options.openPath(next.path)) return undefined;
    pending = null;
    return markRead(next.notificationId);
  };
  return {
    /** A push reached the live runtime: its queries refetch while the app is
     * in front and online, otherwise on the next resume. Approval pushes and
     * other payloads are not notifications and return false. */
    received(data: unknown): boolean {
      if (!notificationPushSignalFromData(data)?.kind) return false;
      invalidate(options.isForeground() && options.isOnline() ? "active" : "none");
      return true;
    },
    /** A tapped push or toast. Returns whether the deeplink is a first-party
     * destination the controller now owns (routed now or once ready). */
    tapped(tap: NotificationPushTap | null | undefined): boolean {
      const path = notificationDeeplinkPath(
        tap?.deeplink,
        options.frontendUrls,
        options.appScheme,
      );
      if (!path) return false;
      pending = { path, notificationId: tap?.notificationId ?? null };
      void flush();
      return true;
    },
    /** Retry a pending tap: on navigator ready, after sign-in, when a guard
     * lifts. Resolves once the row is marked read, or undefined when nothing
     * was dispatched. */
    flush,
    hasPending: () => pending !== null,
  };
}
export type NotificationPushController = ReturnType<
  typeof createNotificationPushController
>;
