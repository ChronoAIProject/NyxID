import { useSyncExternalStore } from "react";
import { useOnline } from "@/hooks/use-online";
import { isConnectionLost, subscribeConnection } from "@/lib/assistant/connection-state";

/** Offline, or NyxID did not answer the last request: the top banner is showing. */
export function useConnectionDown(): boolean {
  const online = useOnline();
  const lost = useSyncExternalStore(subscribeConnection, isConnectionLost, () => false);
  return !online || lost;
}
