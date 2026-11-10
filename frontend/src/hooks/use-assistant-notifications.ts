import { useEffect, useMemo } from "react";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import { useFeature } from "@/hooks/use-feature-flag";
import { livePollInterval, useNyxAgentLiveConnected } from "@/hooks/use-nyxagent-live-status";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { notificationsApi } from "@/lib/assistant/notifications-api";
import type {
  NotificationFilter,
  NotificationList,
  NotificationReadRequest,
  NotificationSourceType,
  NotificationSummary,
} from "@/schemas/assistant-notifications";

type NotificationReadSource = { readonly type: NotificationSourceType; readonly id: string };
import { useAuthStore } from "@/stores/auth-store";

/** Nested under the NyxAgent root so a live `resync` re-reads the feed with everything else. */
export const notificationQueryKeys = {
  root: (userId: string | undefined) => [...nyxBotQueryKeys.root(userId), "notifications"] as const,
  summary: (userId: string | undefined) =>
    [...nyxBotQueryKeys.root(userId), "notifications", "summary"] as const,
  list: (userId: string | undefined, filter: NotificationFilter) =>
    [...nyxBotQueryKeys.root(userId), "notifications", "list", filter] as const,
};

/** Without the live stream the bell re-reads this often; with it, only as a backstop. */
export const NOTIFICATION_SUMMARY_POLL_MS = 60_000;
/** A thread the user is looking at is reported read this long after it opens. */
export const NOTIFICATION_SOURCE_READ_DELAY_MS = 750;

/** The feed exists only with the NyxAgent engine and the default-off flag for this person. */
export function useNotificationsEnabled(): boolean {
  const engine = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
  const notifications = useFeature(FEATURE_FLAG.ASSISTANT_NOTIFICATIONS);
  return engine && notifications;
}

export function useNotificationSummary(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  const live = useNyxAgentLiveConnected();
  return useQuery({
    queryKey: notificationQueryKeys.summary(userId),
    queryFn: () => notificationsApi.summary(),
    enabled: enabled && Boolean(userId),
    retry: false,
    refetchInterval: livePollInterval(NOTIFICATION_SUMMARY_POLL_MS, live),
  });
}

/** Newest first, one page at a time; the first page's `as_of` fences "Mark all as read". */
export function useNotificationList(filter: NotificationFilter, enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useInfiniteQuery({
    queryKey: notificationQueryKeys.list(userId, filter),
    queryFn: ({ pageParam }) => notificationsApi.list({ cursor: pageParam, filter }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: enabled && Boolean(userId),
    retry: false,
  });
}

function markPagesRead(
  data: InfiniteData<NotificationList> | undefined,
  matches: (id: string) => boolean,
): InfiniteData<NotificationList> | undefined {
  if (!data) return data;
  return {
    ...data,
    pages: data.pages.map((page) => ({
      ...page,
      items: page.items.map((item) => (matches(item.id) && !item.read ? { ...item, read: true } : item)),
    })),
  };
}

/**
 * Report notifications as read. Rows named by id turn read at once in every
 * cached page; the server answer then refreshes the badge and the lists.
 */
export function useMarkNotificationsRead() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: NotificationReadRequest) => notificationsApi.markRead(body),
    onMutate: (body) => {
      if (!("ids" in body)) return;
      const ids = new Set(body.ids);
      queryClient.setQueriesData<InfiniteData<NotificationList>>(
        { queryKey: [...notificationQueryKeys.root(userId), "list"] },
        (data) => markPagesRead(data, (id) => ids.has(id)),
      );
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: notificationQueryKeys.root(userId) });
    },
  });
}

export interface UnreadSources {
  /** Threads (including channel chats and automation threads) with unread rows. */
  readonly conversationIds: ReadonlySet<string>;
  readonly groupIds: ReadonlySet<string>;
}

const NO_SOURCES: UnreadSources = {
  conversationIds: new Set(),
  groupIds: new Set(),
};

/**
 * Sidebar markers come from the summary's per-source aggregate, never a
 * second request. A channel chat's source id is its conversation id, and
 * automation sources name their thread through `conversation_id`.
 */
export function unreadSources(summary: NotificationSummary | undefined): UnreadSources {
  if (!summary) return NO_SOURCES;
  const conversationIds = new Set<string>();
  const groupIds = new Set<string>();
  for (const source of summary.by_source) {
    if (source.unread <= 0) continue;
    if (source.conversation_id) conversationIds.add(source.conversation_id);
    switch (source.source_type) {
      case "conversation":
      case "channel_chat":
        conversationIds.add(source.id);
        break;
      case "group":
      case "collaboration_group":
        groupIds.add(source.id);
        break;
      case "trigger":
        break;
    }
  }
  return { conversationIds, groupIds };
}

/**
 * Whether the open thread or group has unread rows. A truncated aggregate
 * cannot say, so any unread row counts then (the read is idempotent).
 */
export function sourceHasUnread(
  summary: NotificationSummary | undefined,
  source: { readonly conversationId?: string; readonly groupId?: string },
): boolean {
  if (!summary || summary.unread_count <= 0) return false;
  if (summary.by_source_truncated) return true;
  const sources = unreadSources(summary);
  return Boolean(
    (source.conversationId && sources.conversationIds.has(source.conversationId)) ||
      (source.groupId && sources.groupIds.has(source.groupId)),
  );
}

/**
 * Opening a thread or group reports its notifications read: debounced, only
 * while the tab is visible, and only when the summary says there is
 * something to mark. A pending decision becomes seen (its badge clears) but
 * stays listed until the card is decided.
 */
export function useNotificationSourceRead({
  conversationId,
  groupId,
  enabled,
}: {
  readonly conversationId?: string;
  readonly groupId?: string;
  readonly enabled: boolean;
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const summary = useNotificationSummary(enabled);
  const { mutate: reportRead } = useMarkNotificationsRead();
  const due = sourceHasUnread(summary.data, { conversationId, groupId });
  const source = useMemo<NotificationReadSource | undefined>(() => {
    if (conversationId) return { type: "conversation", id: conversationId };
    if (groupId) return { type: "group", id: groupId };
    return undefined;
  }, [conversationId, groupId]);
  useEffect(() => {
    if (!enabled || !userId || !source || !due) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let sent = false;
    const schedule = () => {
      if (sent || timer !== undefined || document.visibilityState === "hidden") return;
      timer = setTimeout(() => {
        timer = undefined;
        if (sent || document.visibilityState === "hidden") return;
        sent = true;
        reportRead({ source });
      }, NOTIFICATION_SOURCE_READ_DELAY_MS);
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        if (timer !== undefined) clearTimeout(timer);
        timer = undefined;
      } else {
        schedule();
      }
    };
    schedule();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      if (timer !== undefined) clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [enabled, userId, source, due, reportRead]);
}
