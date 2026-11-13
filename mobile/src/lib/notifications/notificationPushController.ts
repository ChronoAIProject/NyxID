import type { QueryClient } from "@tanstack/react-query";
import { NOTIFICATIONS } from "../../features/notifications/notificationKeys";
import type { NotificationsApi } from "../api/notificationsTransport";
import { notificationPushSignalFromData } from "./notificationPushSignal";

export type NotificationPushControllerOptions = {
  api: Pick<NotificationsApi, "markRead">;
  queryClient: Pick<QueryClient, "invalidateQueries">;
  isForeground: () => boolean;
  isOnline: () => boolean;
};

/** Feed effects only. useNotificationNavigation owns the account-bound tap
 * queue and calls navigated after dispatching a supported destination. */
export function createNotificationPushController(
  options: NotificationPushControllerOptions,
) {
  const invalidate = (refetchType: "active" | "none") =>
    void options.queryClient.invalidateQueries({
      queryKey: NOTIFICATIONS,
      refetchType,
    });
  return {
    received(data: unknown): boolean {
      if (!notificationPushSignalFromData(data)?.kind) return false;
      invalidate(options.isForeground() && options.isOnline() ? "active" : "none");
      return true;
    },
    async navigated(notificationId: string): Promise<void> {
      try {
        await options.api.markRead({ ids: [notificationId] });
      } catch {
        // The row stays unread; opening the thread reads its source later.
      }
      invalidate("active");
    },
  };
}
