import { useEffect, useState, useSyncExternalStore } from "react";
import {
  getLastChatActivityAt,
  subscribeChatActivity,
} from "@/lib/assistant/connect-watch";

function documentVisible(): boolean {
  if (typeof document === "undefined") return true;
  return document.visibilityState !== "hidden";
}

export interface ChatPresence {
  /** The chat tab is in front of the user right now. */
  readonly visible: boolean;
  /** Epoch ms of the last user interaction with the chat; 0 if none yet. */
  readonly lastActivityAt: number;
}

/**
 * Presence signals for background work owned by the chat surface.
 *
 * `visible` gates polling: a hidden tab must not keep hitting the API, and it
 * does not need to — the pending-connect watch refetches on window focus, so
 * coming back from the provider's tab resolves the card immediately rather
 * than after the next interval.
 *
 * `lastActivityAt` is the liveness signal. A user who finished authorizing and
 * went back to chatting about something else is still present, and their
 * pending card should keep waiting rather than time out underneath them.
 */
export function useChatPresence(): ChatPresence {
  const lastActivityAt = useSyncExternalStore(
    subscribeChatActivity,
    getLastChatActivityAt,
    getLastChatActivityAt,
  );
  const [visible, setVisible] = useState(documentVisible);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const onVisibilityChange = () => setVisible(documentVisible());
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () =>
      document.removeEventListener("visibilitychange", onVisibilityChange);
  }, []);

  return { visible, lastActivityAt };
}
