import { useEffect, useState, useSyncExternalStore } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Wifi, WifiOff } from "lucide-react";
import { useOnline } from "@/hooks/use-online";
import { isConnectionLost, subscribeConnection } from "@/lib/assistant/connection-state";
import { cn } from "@/lib/utils";

/** How long "Back online" stays after the connection returns. */
const RECONNECTED_MS = 3000;
/** How often to try again while NyxID is unreachable but the browser is online. */
const RETRY_MS = 5000;

function down(): boolean {
  return !navigator.onLine || isConnectionLost();
}

/**
 * The whole window's connection state, across the top: offline, or NyxID not
 * answering, nothing loads or sends, so it is said once, prominently, rather
 * than as each request's own error. Lists and the open chat refresh by
 * themselves when the connection returns.
 */
export function OfflineBanner() {
  const online = useOnline();
  const unreachable = useSyncExternalStore(subscribeConnection, isConnectionLost, () => false);
  const [reconnected, setReconnected] = useState(false);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let wasDown = down();
    const check = () => {
      const now = down();
      if (now) {
        clearTimeout(timer);
        setReconnected(false);
      } else if (wasDown) {
        clearTimeout(timer);
        setReconnected(true);
        timer = setTimeout(() => setReconnected(false), RECONNECTED_MS);
      }
      wasDown = now;
    };
    window.addEventListener("online", check);
    window.addEventListener("offline", check);
    const unsubscribe = subscribeConnection(check);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("online", check);
      window.removeEventListener("offline", check);
      unsubscribe();
    };
  }, []);
  // Nothing else may be polling, so retry what is on screen; the first answer
  // clears the banner. Offline, the browser's own "online" event does it.
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!online || !unreachable) return;
    const timer = setInterval(() => {
      void queryClient.refetchQueries({ type: "active" });
    }, RETRY_MS);
    return () => clearInterval(timer);
  }, [online, unreachable, queryClient]);
  const isDown = !online || unreachable;
  if (!isDown && !reconnected) return null;
  return (
    <div
      role={isDown ? "alert" : "status"}
      className={cn(
        // Neutral surface; only the icon carries the state's colour.
        "flex shrink-0 items-center justify-center gap-2 border-b border-hairline bg-overlay-strong px-4 py-2 text-12 font-medium text-foreground",
      )}
    >
      {isDown ? (
        <WifiOff aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-warning" />
      ) : (
        <Wifi aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-success" />
      )}
      <span>
        {/* Nothing is persisted: what shows is what this tab already loaded. */}
        {!online
          ? "You're offline. NyxBot can't send, run or refresh until you reconnect. You're seeing what was already loaded."
          : unreachable
            ? "Can't reach NyxID. NyxBot can't send, run or refresh until the connection is back. You're seeing what was already loaded."
            : "Back online."}
      </span>
    </div>
  );
}
