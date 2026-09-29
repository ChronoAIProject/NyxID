import { useSyncExternalStore } from "react";
import {
  getNyxAgentLiveStatus,
  subscribeNyxAgentLiveStatus,
} from "@/lib/assistant/nyxagent-live";

/** While the live stream is open, a poll that is still wanted runs this slowly. */
export const LIVE_BACKSTOP_MS = 30_000;

export function useNyxAgentLiveStatus() {
  return useSyncExternalStore(
    subscribeNyxAgentLiveStatus,
    getNyxAgentLiveStatus,
    getNyxAgentLiveStatus,
  );
}

/** True while NyxID pushes changes, so polls can back off. */
export function useNyxAgentLiveConnected() {
  return useNyxAgentLiveStatus() === "live";
}

/** A poll interval, slowed to a backstop while NyxID pushes changes. */
export function livePollInterval(interval: number | false, live: boolean): number | false {
  if (interval === false || !live) return interval;
  return Math.max(interval, LIVE_BACKSTOP_MS);
}
