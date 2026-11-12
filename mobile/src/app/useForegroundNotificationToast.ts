import { useEffect, useRef, useState } from "react";
import type { ToastState } from "../components/ToastOverlay";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../lib/auth/authAccount";
import { setForegroundNotificationHandler } from "../lib/notifications/pushNotifications";

/** Foreground content and retained actions belong to the account at arrival. */
export function useForegroundNotificationToast(
  openChallenge: (id: string) => void,
  open: (url: string) => void,
) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const [toastKey, setToastKey] = useState(0);
  const revision = useRef(0);
  useEffect(() => {
    let mounted = true;
    const unsubscribe = subscribeAuthAccount(() => {
      revision.current++;
      setToast(null);
      setToastKey(revision.current);
    });
    const unregister = setForegroundNotificationHandler((notification) => {
      const account = getAuthAccountId();
      const arrival = revision.current;
      const message = notification.title ?? notification.body;
      if (!account || !message) return;
      const action = notification.challengeId
        ? () => openChallenge(notification.challengeId!)
        : notification.url
          ? () => open(notification.url!)
          : undefined;
      setToast({
        message,
        kind: "info",
        action: action
          ? {
              label: notification.challengeId ? "Review" : "Open",
              onPress: () => {
                if (
                  mounted &&
                  getAuthAccountId() === account &&
                  revision.current === arrival
                )
                  action();
              },
            }
          : undefined,
      });
    });
    return () => {
      mounted = false;
      unsubscribe();
      unregister();
    };
  }, [openChallenge, open]);
  return { toast, toastKey };
}
