import { useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { getCurrentUserProfileRequest } from "../../lib/api/http";
import { notificationsApi } from "../../lib/api/notificationsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupsLive } from "../groups/useGroupsLive";
import {
  notificationFilterKinds,
  type NotificationFilter,
  type NotificationPushPreferences,
  type NotificationReadScope,
} from "./notificationContracts";
import {
  notificationBadge,
  notificationMarkers,
  unreadForSource,
} from "./notificationPresentation";

import { NOTIFICATIONS, notificationKeys } from "./notificationKeys";

export { NOTIFICATIONS, notificationKeys };
export const NOTIFICATIONS_FLAG = "assistant:notifications";
/** Notifications only exist for NyxAgent accounts; the API gates on both. */
export const NYXAGENT_ENGINE_FLAG = "assistant:nyxagent-engine";
/** Backstop while the live stream is down; live events refresh at once. */
export const NOTIFICATION_POLL_MS = 30_000;
/** A thread must stay in front this long before it counts as seen. */
export const VISIBLE_THREAD_READ_DELAY_MS = 1_500;
const LIVE_COALESCE_MS = 150;
const PROFILE_KEY = ["account", "profile"] as const;

/** The per-person `assistant:notifications` flag together with the NyxAgent
 * engine flag, read from the profile the rest of the workspace already
 * caches; no extra request when it is fresh. With either flag off the bell,
 * feed, settings switches and their requests stay away, as on the web. */
export function useNotificationsEnabled(active = true): boolean {
  const profile = useQuery({
    queryKey: PROFILE_KEY,
    queryFn: ({ signal }) => getCurrentUserProfileRequest(signal),
    enabled: active,
    staleTime: 60_000,
    retry: false,
  });
  const features = profile.data?.capabilities?.enabled_features;
  return (
    !!features &&
    features.includes(NOTIFICATIONS_FLAG) &&
    features.includes(NYXAGENT_ENGINE_FLAG)
  );
}

/** Refresh only while this screen is focused, the app is in front and online. */
export function useNotificationRuntime() {
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const { isConnected } = useNetworkStatus();
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (value) =>
      setForeground(value === "active"),
    );
    return () => subscription.remove();
  }, []);
  return { active: focused && foreground, online: isConnected };
}

/** Shares the workspace live stream; a notification frame or resync marks
 * every notification query stale. Returns whether the stream is live. */
export function useNotificationsLive(enabled: boolean): boolean {
  const client = useQueryClient();
  const epoch = getAuthEpoch();
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const live = useGroupsLive(
    epoch,
    enabled,
    () => epoch === getAuthEpoch(),
    (event) => {
      if (
        event.type !== "notification" &&
        event.type !== "resync" &&
        event.type !== "ready"
      )
        return;
      if (timer.current) return;
      timer.current = setTimeout(() => {
        timer.current = undefined;
        if (epoch === getAuthEpoch())
          void client.invalidateQueries({ queryKey: NOTIFICATIONS });
      }, LIVE_COALESCE_MS);
    },
  );
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      timer.current = undefined;
    },
    [epoch, enabled],
  );
  return live;
}

export function useNotificationSummary(
  enabled: boolean,
  active: boolean,
  live = false,
) {
  const epoch = getAuthEpoch();
  return useQuery({
    queryKey: notificationKeys.summary(epoch),
    queryFn: ({ signal }) => notificationsApi.summary(signal),
    enabled: enabled && active,
    refetchInterval: livePollInterval(
      enabled && active ? NOTIFICATION_POLL_MS : false,
      live,
    ),
    retry: false,
  });
}

export function useNotificationFeed(
  filter: NotificationFilter,
  enabled: boolean,
  active: boolean,
  live = false,
) {
  const epoch = getAuthEpoch();
  return useInfiniteQuery({
    queryKey: notificationKeys.feed(epoch, filter),
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      notificationsApi.list(
        {
          cursor: pageParam,
          kinds: notificationFilterKinds(filter),
          // Needs you asks the server for waiting decisions only; every other
          // filter keeps read rows and resolved card history in the feed.
          pendingOnly: filter === "needs_you",
        },
        signal,
      ),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: enabled && active,
    refetchInterval: livePollInterval(
      enabled && active ? NOTIFICATION_POLL_MS : false,
      live,
    ),
    retry: false,
  });
}

export function useMarkNotificationsRead() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (scope: NotificationReadScope) =>
      notificationsApi.markRead(scope),
    onSuccess: () => client.invalidateQueries({ queryKey: NOTIFICATIONS }),
  });
}

export function useNotificationPushPreferences(enabled: boolean) {
  const epoch = getAuthEpoch();
  return useQuery({
    queryKey: notificationKeys.preferences(epoch),
    queryFn: ({ signal }) => notificationsApi.pushPreferences(signal),
    enabled,
    retry: false,
  });
}

export function useUpdateNotificationPushPreferences() {
  const client = useQueryClient();
  const epoch = getAuthEpoch();
  return useMutation({
    mutationFn: (push: NotificationPushPreferences) =>
      notificationsApi.updatePushPreferences(push),
    onSuccess: (saved) => {
      client.setQueryData(notificationKeys.preferences(epoch), saved);
    },
  });
}

/** Bell state for the sidebar and chat header: flag, counts, badge text and
 * the per-thread markers the sidebar rows draw from this one summary. */
export function useNotificationIndicator(active: boolean) {
  const enabled = useNotificationsEnabled(active);
  const live = useNotificationsLive(enabled && active);
  const summary = useNotificationSummary(enabled, active, live);
  const unread = summary.data?.unread_count ?? 0;
  const needsDecision = summary.data?.needs_decision_count ?? 0;
  const markers = useMemo(
    () => notificationMarkers(summary.data),
    [summary.data],
  );
  return {
    enabled,
    live,
    unread,
    needsDecision,
    badge: notificationBadge(unread),
    markers,
  };
}

export type VisibleNotificationSource = {
  type: "conversation" | "group";
  id: string;
};

/** Reads sent for one unchanged unread count before the thread stops asking
 * (the server did not clear it); a changed count starts a fresh budget. */
export const VISIBLE_THREAD_MAX_EXACT_READS = 3;

type VisibleReadView = {
  exact: number;
  anyUnread: number;
  truncated: boolean;
  dataUpdatedAt: number;
};
/** What one open of a thread has already read; reset by a new open only. */
type VisibleReadState = {
  exactReads: number;
  lastReadExact?: number;
  lastReadAt?: number;
  fallbackUsed: boolean;
};
const freshVisibleReadState = (): VisibleReadState => ({
  exactReads: 0,
  fallbackUsed: false,
});

function visibleReadDecision(
  view: VisibleReadView,
  state: VisibleReadState,
): { read: boolean; state: VisibleReadState } {
  if (view.exact > 0) {
    // This summary was already read; wait for the server's next word.
    if (state.lastReadAt === view.dataUpdatedAt) return { read: false, state };
    const sameCount = state.lastReadExact === view.exact;
    if (sameCount && state.exactReads >= VISIBLE_THREAD_MAX_EXACT_READS)
      return { read: false, state };
    return {
      read: true,
      state: {
        ...state,
        exactReads: sameCount ? state.exactReads + 1 : 1,
        lastReadExact: view.exact,
        lastReadAt: view.dataUpdatedAt,
      },
    };
  }
  // Nothing is attributed to this thread now: a later arrival starts fresh.
  const cleared =
    state.lastReadExact === undefined
      ? state
      : { ...state, exactReads: 0, lastReadExact: undefined, lastReadAt: undefined };
  if (view.truncated && view.anyUnread > 0 && !cleared.fallbackUsed)
    return { read: true, state: { ...cleared, fallbackUsed: true } };
  return { read: false, state: cleared };
}

/**
 * Opening a thread marks its rows read (SPEC D6). The thread must stay in
 * front for the debounce; that deadline is armed once per open or resume
 * and survives summary refreshes, so a busy feed cannot postpone it. From
 * then on every summary is judged at once: an exact unread count reads,
 * re-reads when the count changes, and asks at most
 * `VISIBLE_THREAD_MAX_EXACT_READS` times for a count the server leaves
 * unchanged. A truncated summary with unexact counts reads once per open.
 * An open is one thread under one account while the screen stays focused:
 * a background pause keeps what was already read, while a navigation blur
 * and return, another thread, another account or a fresh mount start over
 * (the web's tab-visibility behavior). Decisions become seen, never
 * resolved, server-side.
 */
export function useVisibleThreadRead(
  source: VisibleNotificationSource | undefined,
  enabled: boolean,
  active: boolean,
) {
  const summary = useNotificationSummary(enabled, active);
  const markRead = useMarkNotificationsRead();
  const focused = useIsFocused();
  const mutate = useRef(markRead.mutate);
  mutate.current = markRead.mutate;
  const sourceKey = source ? `${source.type}:${source.id}` : undefined;
  const identity = sourceKey ? `${getAuthEpoch()}:${sourceKey}` : undefined;
  const view: VisibleReadView = {
    exact: source ? unreadForSource(summary.data, source.type, source.id) : 0,
    anyUnread: summary.data?.unread_count ?? 0,
    truncated: summary.data?.by_source_truncated ?? false,
    dataUpdatedAt: summary.dataUpdatedAt,
  };
  const latest = useRef({ view, source });
  latest.current = { view, source };
  const open = useRef<{
    identity?: string;
    focused: boolean;
    state: VisibleReadState;
    due: boolean;
    /** Counts opens so the deadline re-arms for each one. */
    generation: number;
  }>({ identity, focused, state: freshVisibleReadState(), due: false, generation: 0 });
  if (open.current.identity !== identity || (focused && !open.current.focused))
    open.current = {
      identity,
      focused,
      state: freshVisibleReadState(),
      due: false,
      generation: open.current.generation + 1,
    };
  else open.current.focused = focused;
  const generation = open.current.generation;
  const judge = useRef(() => {});
  judge.current = () => {
    const current = open.current;
    const target = latest.current.source;
    if (!current.due || !target) return;
    const next = visibleReadDecision(latest.current.view, current.state);
    current.state = next.state;
    if (next.read) mutate.current({ source: { type: target.type, id: target.id } });
  };
  // The deadline: armed for each open when the thread is in front, dropped on pause.
  useEffect(() => {
    if (!enabled || !active || !identity) return;
    const timer = setTimeout(() => {
      open.current.due = true;
      judge.current();
    }, VISIBLE_THREAD_READ_DELAY_MS);
    return () => {
      clearTimeout(timer);
      open.current.due = false;
    };
  }, [enabled, active, identity, generation]);
  // After the deadline every summary refresh is judged immediately.
  const { exact, anyUnread, truncated, dataUpdatedAt } = view;
  useEffect(() => {
    if (enabled && active) judge.current();
  }, [enabled, active, identity, generation, exact, anyUnread, truncated, dataUpdatedAt]);
}
