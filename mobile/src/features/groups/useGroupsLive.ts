import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import { AppState } from "react-native";
import { onlineManager, useQueryClient } from "@tanstack/react-query";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { requestAuthenticatedResponse } from "../../lib/api/http";
import type { AssistantLiveEvent } from "../../lib/api/assistantLiveTransport";
import { runAssistantLive } from "../../lib/api/assistantLiveTransport";
import { createGroupLiveSession } from "./groupLiveSession";

const subscribeOnline = (listener: () => void) =>
  onlineManager.subscribe(listener);
const onlineSnapshot = () => onlineManager.isOnline();

// List and transcript observers share one authenticated native stream.
const session = createGroupLiveSession({
  available: (epoch) =>
    epoch === getAuthEpoch() &&
    AppState.currentState === "active" &&
    onlineManager.isOnline(),
  run: ({ signal, onStatus, onEvent }) => {
    const epoch = getAuthEpoch();
    return runAssistantLive({
      signal,
      onStatus,
      onEvent,
      connect: (requestSignal) => {
        if (
          epoch !== getAuthEpoch() ||
          AppState.currentState !== "active" ||
          !onlineManager.isOnline()
        )
          throw new Error("Crew notification subscription is inactive.");
        return requestAuthenticatedResponse("/assistant/nyxagent/live", {
          method: "GET",
          headers: { Accept: "text/event-stream" },
          signal: requestSignal,
        });
      },
    });
  },
});

/** Focused foreground workspace lists and crew views share live notifications. */
export function useGroupsLive(
  epoch: number,
  active: boolean,
  available: () => boolean,
  onEvent?: (event: AssistantLiveEvent) => void,
) {
  const client = useQueryClient();
  // NetInfo's initially unknown state may differ from useNetworkStatus. Observe
  // the actual request gate so a later online transition acquires the stream.
  const online = useSyncExternalStore(
    subscribeOnline,
    onlineSnapshot,
    () => false,
  );
  const enabled = active && online;
  const current = useRef({ enabled, available, onEvent });
  current.current = { enabled, available, onEvent };
  const participating = useCallback(
    () => current.current.enabled && current.current.available(),
    [],
  );
  useEffect(() => {
    if (!enabled) return;
    return session.acquire(client, epoch, participating);
  }, [client, epoch, enabled, participating]);
  useEffect(() => {
    if (!enabled || !onEvent) return;
    return session.subscribeEvents(client, epoch, (event) => {
      if (participating() && epoch === getAuthEpoch())
        current.current.onEvent?.(event);
    });
  }, [client, epoch, enabled, !!onEvent, participating]);
  const subscribe = useCallback(
    (listener: () => void) => session.subscribe(listener),
    [],
  );
  const snapshot = useCallback(
    () => session.getStatus(client, epoch),
    [client, epoch],
  );
  const status = useSyncExternalStore(subscribe, snapshot, () => "idle");
  return enabled && status === "live";
}
