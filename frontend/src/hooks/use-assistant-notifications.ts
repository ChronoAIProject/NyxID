import { useEffect, useMemo, useRef } from "react";
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
import { useAuthStore } from "@/stores/auth-store";

type NotificationReadSource = { readonly type: NotificationSourceType; readonly id: string };

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

/**
 * The server may answer a page with no visible rows but a cursor: its bounded
 * row page held only hidden or revoked sources. After this many such pages in
 * a row the feed stops loading further pages by itself; Load more still works.
 */
export const NOTIFICATION_EMPTY_PAGE_AUTOLOAD_LIMIT = 5;

/** How many pages at the end of the loaded list came back without rows. */
export function trailingEmptyPages(pages: readonly Pick<NotificationList, "items">[]): number {
  let count = 0;
  for (let index = pages.length - 1; index >= 0 && pages[index]!.items.length === 0; index -= 1) {
    count += 1;
  }
  return count;
}

/** Newest first, one page at a time; the first page's `as_of` fences "Mark all as read". */
export function useNotificationList(filter: NotificationFilter, enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useInfiniteQuery({
    queryKey: notificationQueryKeys.list(userId, filter),
    queryFn: ({ pageParam }) => notificationsApi.list({ cursor: pageParam, filter }),
    initialPageParam: undefined as string | undefined,
    // A cursor equal to the one just used would fetch the same page forever.
    getNextPageParam: (page, _pages, lastPageParam) =>
      page.next_cursor && page.next_cursor !== lastPageParam ? page.next_cursor : undefined,
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

/** How the summary speaks for the open source. */
export interface SourceStanding {
  /** `exact`: the source is listed; `fallback`: only a truncated total says something is unread. */
  readonly kind: "none" | "exact" | "fallback";
  /** The listed unread count (0 for `fallback` and `none`). */
  readonly unread: number;
}

/**
 * What the summary says about the open thread or group. A truncated
 * aggregate cannot name sources, so any unread row counts then (the read
 * is idempotent); an exact listing names the source and its count.
 */
export function sourceStanding(
  summary: NotificationSummary | undefined,
  source: { readonly conversationId?: string; readonly groupId?: string },
): SourceStanding {
  if (!summary || summary.unread_count <= 0) return { kind: "none", unread: 0 };
  let unread = 0;
  for (const entry of summary.by_source) {
    if (entry.unread <= 0) continue;
    const matches =
      (source.conversationId &&
        (entry.conversation_id === source.conversationId ||
          ((entry.source_type === "conversation" || entry.source_type === "channel_chat") &&
            entry.id === source.conversationId))) ||
      (source.groupId &&
        (entry.source_type === "group" || entry.source_type === "collaboration_group") &&
        entry.id === source.groupId);
    if (matches) unread += entry.unread;
  }
  if (unread > 0) return { kind: "exact", unread };
  return summary.by_source_truncated ? { kind: "fallback", unread: 0 } : { kind: "none", unread: 0 };
}

/** Whether the open thread or group has unread rows (see `sourceStanding`). */
export function sourceHasUnread(
  summary: NotificationSummary | undefined,
  source: { readonly conversationId?: string; readonly groupId?: string },
): boolean {
  return sourceStanding(summary, source).kind !== "none";
}

/**
 * Consecutive reads of one open source while the summary keeps listing it
 * unread. Normally one read clears it; a second covers activity that landed
 * between the read and the refreshed summary. Beyond this, only a changed
 * count re-arms, so a server that never clears cannot drive a read loop.
 */
export const NOTIFICATION_SOURCE_READ_MAX_CONSECUTIVE = 3;

interface SourceReadState {
  key?: string;
  /** Deadline of the pending debounced read, so re-renders never extend it. */
  dueAt?: number;
  /** The summary observation (`dataUpdatedAt`) the last read was based on; only another observation can re-arm. */
  readObservedAt: number;
  /** Reads since the summary last reported the source clear. */
  consecutive: number;
  /** The listed count at the last read, for the bounded re-arm rule. */
  lastUnread?: number;
  /** A truncated aggregate earns one read per open source. */
  fallbackSent: boolean;
}

/**
 * Opening a thread or group reports its notifications read: debounced, only
 * while the tab is visible, and only when the summary says there is
 * something to mark. A pending decision becomes seen (its badge clears) but
 * stays listed until the card is decided.
 *
 * Re-arming follows the summary, not a boolean: every new summary observation
 * that still lists the open source unread (new activity, or activity that
 * raced the read) schedules another read, bounded by
 * `NOTIFICATION_SOURCE_READ_MAX_CONSECUTIVE` until the source clears once. A
 * truncated aggregate cannot name sources, so it earns one read per open.
 */
export function useNotificationSourceRead({
  conversationId,
  groupId,
  enabled,
  delayMs = NOTIFICATION_SOURCE_READ_DELAY_MS,
}: {
  readonly conversationId?: string;
  readonly groupId?: string;
  readonly enabled: boolean;
  readonly delayMs?: number;
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const summary = useNotificationSummary(enabled);
  const { mutateAsync: reportRead } = useMarkNotificationsRead();
  const source = useMemo<NotificationReadSource | undefined>(() => {
    if (conversationId) return { type: "conversation", id: conversationId };
    if (groupId) return { type: "group", id: groupId };
    return undefined;
  }, [conversationId, groupId]);
  const sourceKey = source && userId ? `${userId}:${source.type}:${source.id}` : undefined;
  const standing = sourceStanding(summary.data, { conversationId, groupId });
  const observedAt = summary.dataUpdatedAt;
  const state = useRef<SourceReadState>({ readObservedAt: 0, consecutive: 0, fallbackSent: false });
  useEffect(() => {
    const current = state.current;
    if (current.key !== sourceKey) {
      // Another thread or crew (or another person): a fresh open.
      state.current = { key: sourceKey, readObservedAt: 0, consecutive: 0, fallbackSent: false };
    }
    const memory = state.current;
    if (!enabled || !userId || !source || !sourceKey) return;
    if (standing.kind === "none") {
      // The server confirmed the source clear: later activity starts a new run.
      memory.consecutive = 0;
      memory.lastUnread = undefined;
      memory.dueAt = undefined;
      return;
    }
    if (standing.kind === "fallback") {
      if (memory.fallbackSent) return;
    } else {
      // Only another summary observation than the one already read can prove
      // new activity (clocks are not compared: an in-memory round trip can
      // finish within the millisecond the read was sent).
      if (observedAt === memory.readObservedAt) return;
      if (
        memory.consecutive >= NOTIFICATION_SOURCE_READ_MAX_CONSECUTIVE &&
        memory.lastUnread === standing.unread
      ) {
        return;
      }
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    // One read per observed summary: a tab toggle after sending must not resend.
    let sent = false;
    const send = () => {
      sent = true;
      memory.dueAt = undefined;
      if (standing.kind === "fallback") {
        memory.fallbackSent = true;
      } else {
        memory.consecutive += 1;
        memory.lastUnread = standing.unread;
      }
      memory.readObservedAt = observedAt;
      void reportRead({ source }).catch(() => undefined);
    };
    const schedule = () => {
      if (disposed || sent || timer !== undefined || document.visibilityState === "hidden") return;
      // A refreshed summary keeps the original deadline instead of extending it.
      memory.dueAt ??= Date.now() + delayMs;
      timer = setTimeout(() => {
        timer = undefined;
        if (disposed || document.visibilityState === "hidden") return;
        send();
      }, Math.max(0, memory.dueAt - Date.now()));
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
      disposed = true;
      if (timer !== undefined) clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [enabled, userId, source, sourceKey, standing.kind, standing.unread, observedAt, delayMs, reportRead]);
}
