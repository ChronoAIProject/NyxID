import { useEffect, useRef, useState } from "react";
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
import { notificationBadge, unreadForSource } from "./notificationPresentation";

export const NOTIFICATIONS = ["workspace", "notifications"] as const;
export const NOTIFICATIONS_FLAG = "assistant:notifications";
/** Backstop while the live stream is down; live events refresh at once. */
export const NOTIFICATION_POLL_MS = 30_000;
/** A thread must stay in front this long before it counts as seen. */
export const VISIBLE_THREAD_READ_DELAY_MS = 1_500;
const LIVE_COALESCE_MS = 150;
const PROFILE_KEY = ["account", "profile"] as const;

export const notificationKeys = {
  summary: (epoch: number) => [...NOTIFICATIONS, "summary", epoch] as const,
  feed: (epoch: number, filter: NotificationFilter) =>
    [...NOTIFICATIONS, "feed", epoch, filter] as const,
  preferences: (epoch: number) =>
    [...NOTIFICATIONS, "preferences", epoch] as const,
};

/** The per-person `assistant:notifications` flag, read from the profile the
 * rest of the workspace already caches; no extra request when it is fresh. */
export function useNotificationsEnabled(active = true): boolean {
  const profile = useQuery({
    queryKey: PROFILE_KEY,
    queryFn: ({ signal }) => getCurrentUserProfileRequest(signal),
    enabled: active,
    staleTime: 60_000,
    retry: false,
  });
  return (
    profile.data?.capabilities?.enabled_features?.includes(NOTIFICATIONS_FLAG) ??
    false
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
    refetchInterval: livePollInterval(active ? NOTIFICATION_POLL_MS : false, live),
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
    refetchInterval: livePollInterval(active ? NOTIFICATION_POLL_MS : false, live),
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

/** Bell state for the sidebar and chat header: flag, counts and badge text. */
export function useNotificationIndicator(active: boolean) {
  const enabled = useNotificationsEnabled(active);
  const live = useNotificationsLive(enabled && active);
  const summary = useNotificationSummary(enabled, active, live);
  const unread = summary.data?.unread_count ?? 0;
  const needsDecision = summary.data?.needs_decision_count ?? 0;
  return { enabled, live, unread, needsDecision, badge: notificationBadge(unread) };
}

export type VisibleNotificationSource = {
  type: "conversation" | "group";
  id: string;
};

/**
 * Opening a thread marks its rows read (SPEC D6): once it has stayed in front
 * for the debounce, send one source read. The summary attributes channel and
 * automation rows to their thread, so exact counts re-arm the read; when the
 * bounded summary is truncated a thread reads once per open while anything is
 * unread. Decisions become seen, never resolved, server-side.
 */
export function useVisibleThreadRead(
  source: VisibleNotificationSource | undefined,
  enabled: boolean,
  active: boolean,
) {
  const summary = useNotificationSummary(enabled, active);
  const markRead = useMarkNotificationsRead();
  const mutate = useRef(markRead.mutate);
  mutate.current = markRead.mutate;
  const attempted = useRef<string | undefined>(undefined);
  const exact = source
    ? unreadForSource(summary.data, source.type, source.id)
    : 0;
  const anyUnread = summary.data?.unread_count ?? 0;
  const truncated = summary.data?.by_source_truncated ?? false;
  const token = !source
    ? undefined
    : exact > 0
      ? `${source.type}:${source.id}:exact:${summary.dataUpdatedAt}`
      : truncated && anyUnread > 0
        ? `${source.type}:${source.id}:any`
        : undefined;
  const sourceType = source?.type;
  const sourceId = source?.id;
  useEffect(() => {
    if (!enabled || !active || !token || !sourceType || !sourceId) return;
    if (attempted.current === token) return;
    const timer = setTimeout(() => {
      attempted.current = token;
      mutate.current({ source: { type: sourceType, id: sourceId } });
    }, VISIBLE_THREAD_READ_DELAY_MS);
    return () => clearTimeout(timer);
  }, [enabled, active, token, sourceType, sourceId]);
}
