import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  Pressable,
  RefreshControl,
  StyleSheet,
  View,
} from "react-native";
import {
  AtSign,
  Bot,
  CalendarClock,
  AlertTriangle,
  CircleCheck,
  CircleHelp,
  MessageSquare,
  Bell,
  UserRound,
  type LucideIcon,
} from "lucide-react-native";
import type { WorkspaceScreenProps } from "../../app/navigationTypes";
import {
  useOpenWorkspaceChat,
  useWorkspaceBack,
} from "../../app/WorkspaceHeader";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import { ChannelPlatformGlyph } from "../../components/ChannelPlatformGlyph";
import { SegmentControl } from "../../components/SegmentControl";
import { SkeletonRows } from "../../components/Skeleton";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { firstPartyFrontendUrls } from "../../lib/env";
import { fonts, radius, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import {
  NOTIFICATION_FILTERS,
  NOTIFICATION_FILTER_LABELS,
  notificationFilterKeeps,
  type AssistantNotification,
  type NotificationFilter,
  type NotificationKind,
} from "./notificationContracts";
import {
  notificationAccessibilityLabel,
  notificationCountLabel,
  notificationPending,
  notificationTarget,
  notificationTime,
  notificationTitle,
  notificationSentence,
  notificationFilterCount,
  notificationContextLabel,
  notificationTone,
  type NotificationTone,
} from "./notificationPresentation";
import {
  useMarkNotificationsRead,
  useNotificationFeed,
  useNotificationRuntime,
  useNotificationsEnabled,
  useNotificationsLive,
  useNotificationSummary,
} from "./useNotifications";

const KIND_ICON: Record<NotificationKind, LucideIcon> = {
  automation_result: CircleCheck,
  automation_problem: AlertTriangle,
  needs_decision: CircleHelp,
  agent_update: Bot,
  chat_update: MessageSquare,
  group_mention: AtSign,
};

export const NOTIFICATION_EMPTY_COPY: Record<NotificationFilter, string> = {
  all: "You're all caught up.",
  needs_you: "Nothing is waiting for your decision.",
  automations: "No automation results yet.",
  chats: "No chat updates yet.",
};

/**
 * The server may filter a whole page (hidden or revoked sources) and still
 * hand back a cursor. An empty list then keeps walking on its own, one hop
 * per distinct cursor and at most this many hops, so a later visible row
 * stays reachable; beyond that the list offers Load more instead of looping.
 */
export const AUTO_EMPTY_PAGE_HOPS = 5;
export function nextEmptyPageHop(
  cursor: string | null | undefined,
  followed: readonly string[],
  limit = AUTO_EMPTY_PAGE_HOPS,
): boolean {
  return !!cursor && !followed.includes(cursor) && followed.length < limit;
}

export function notificationError(error: unknown): string {
  const message = (error as { message?: unknown } | null)?.message;
  return typeof message === "string" && message.trim()
    ? message
    : "Could not load notifications. Try again.";
}

/** Merge pages, drop duplicates a refetch can repeat, apply the client-side
 * part of a filter (Needs you hides resolved cards). */
export function notificationRows(
  pages: readonly { items: readonly AssistantNotification[] }[] | undefined,
  filter: NotificationFilter,
): AssistantNotification[] {
  const seen = new Set<string>();
  const rows: AssistantNotification[] = [];
  for (const page of pages ?? []) {
    for (const item of page.items) {
      if (seen.has(item.id) || !notificationFilterKeeps(filter, item)) continue;
      seen.add(item.id);
      rows.push(item);
    }
  }
  return rows;
}

function toneColors(colors: ThemeColors, tone: NotificationTone) {
  switch (tone) {
    case "warning":
      return colors.warningTone;
    case "danger":
      return colors.dangerTone;
    case "success":
      return colors.successTone;
    default:
      return { bg: colors.cardSoft, text: colors.textMuted, border: colors.border };
  }
}

function Chip({
  label,
  tone,
  colors,
  styles,
}: {
  label: string;
  tone: NotificationTone;
  colors: ThemeColors;
  styles: ReturnType<typeof createStyles>;
}) {
  const palette = toneColors(colors, tone);
  return (
    <View style={[styles.chip, { backgroundColor: palette.bg }]}>
      <Text style={[styles.chipText, { color: palette.text }]}>{label}</Text>
    </View>
  );
}

export function NotificationRow({
  item,
  now,
  onPress,
}: {
  item: AssistantNotification;
  now: number;
  onPress: (item: AssistantNotification) => void;
}) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const tone = notificationTone(item.kind);
  const palette = toneColors(colors, tone);
  const Icon = KIND_ICON[item.kind];
  const sentence = notificationSentence(item);
  const contextLabel = notificationContextLabel(item);
  const agents = (item.participants ?? [])
    .filter((actor) => actor.type === "agent" && actor.id)
    .map((actor) => ({
      id: actor.id!,
      name: actor.name,
      display_name: actor.display_name,
      kind: actor.kind ?? ("unknown" as const),
    }));
  const Fallback =
    item.actor?.type === "automation"
      ? CalendarClock
      : item.actor?.type === "person"
        ? UserRound
        : Bell;
  const pending = notificationPending(item);
  const count = notificationCountLabel(item.count);
  const time = notificationTime(item.updated_at, now);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={notificationAccessibilityLabel(item, now)}
      accessibilityState={{ selected: !item.read }}
      onPress={() => onPress(item)}
      testID={`notification-${item.id}`}
      style={({ pressed }) => [
        styles.row,
        {
          backgroundColor: pressed
            ? colors.navActive
            : !item.read
              ? colors.cardSoft
              : "transparent",
        },
      ]}
    >
      <View
        testID={`notification-avatar-${agents.length ? "crew" : (item.actor?.type ?? "system")}`}
        style={styles.avatar}
      >
        {agents.length ? (
          <AgentAvatarStack agents={agents} size="lg" surface={colors.bg} />
        ) : item.actor?.type === "agent" ? (
          <AgentAvatar
            agent={{
              id: item.actor.id ?? item.source.agent_id ?? item.actor.name,
              name: item.actor.name,
              display_name: item.actor.display_name,
              kind: item.actor.kind ?? "unknown",
            }}
            size="lg"
          />
        ) : (
          <View
            style={[styles.iconTile, { backgroundColor: colors.navActive }]}
          >
            <Fallback size={18} color={colors.textMuted} />
          </View>
        )}
        <View
          testID={`notification-badge-${item.kind}`}
          style={[
            styles.kindBadge,
            { backgroundColor: colors.bg, borderColor: colors.bg },
          ]}
        >
          {item.kind === "chat_update" && item.platform ? (
            <ChannelPlatformGlyph platform={item.platform} size={12} color={palette.text} />
          ) : (
            <Icon testID={`notification-kind-icon-${item.kind}`} size={12} color={palette.text} />
          )}
        </View>
      </View>
      <View style={styles.body}>
        <View style={styles.titleLine}>
          <Text
            numberOfLines={3}
            style={[
              styles.title,
              { color: colors.textPrimary },
              !item.read && styles.titleUnread,
            ]}
          >
            {item.actor ? (
              <>
                <Text style={styles.emphasis}>{sentence.actor}</Text>
                {sentence.action ? ` ${sentence.action}` : ""}
                {sentence.target ? (
                  <>
                    {" "}
                    <Text style={styles.emphasis}>{sentence.target}</Text>
                  </>
                ) : null}
                {sentence.platform ? ` ${sentence.platform}` : ""}
              </>
            ) : (
              notificationTitle(item)
            )}
          </Text>
        </View>
        {item.preview ? (
          <Text
            numberOfLines={2}
            style={[styles.preview, { color: colors.textSecondary }]}
          >
            {item.preview}
          </Text>
        ) : null}
        <View style={styles.metaLine}>
          {contextLabel ? (
            <View
              testID="notification-context"
              style={[styles.sourceChip, { backgroundColor: colors.navActive }]}
            >
              <Text
                numberOfLines={1}
                style={[styles.time, { color: colors.textSecondary }]}
              >
                {contextLabel}
              </Text>
            </View>
          ) : null}
          {time ? (
            <Text style={[styles.time, { color: colors.textMuted }]}>
              {time}
            </Text>
          ) : null}
        </View>
        {count || pending ? (
          <View style={styles.metaLine}>
            {count ? (
              <Chip
                label={count}
                tone="default"
                colors={colors}
                styles={styles}
              />
            ) : null}
            {pending ? (
              <Chip
                label="Needs you"
                tone="warning"
                colors={colors}
                styles={styles}
              />
            ) : null}
          </View>
        ) : null}
      </View>
    </Pressable>
  );
}

export function NotificationsScreen({
  navigation,
}: WorkspaceScreenProps<"Notifications">) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  useWorkspaceBack({
    active: false,
    defaultRole: "menu",
    onBack: navigation.goBack,
  });
  const runtime = useNotificationRuntime();
  const enabled = useNotificationsEnabled(runtime.active);
  const live = useNotificationsLive(enabled && runtime.active);
  const [filter, setFilter] = useState<NotificationFilter>("all");
  const feed = useNotificationFeed(filter, enabled, runtime.active, live);
  const summary = useNotificationSummary(enabled, runtime.active, live);
  const markRead = useMarkNotificationsRead();
  const openChat = useOpenWorkspaceChat();
  const now = Date.now();
  const rows = useMemo(
    () => notificationRows(feed.data?.pages, filter),
    [feed.data, filter],
  );
  const asOf = feed.data?.pages[0]?.as_of;
  const unread = summary.data?.unread_count ?? 0;
  // Nothing visible yet while the server still has pages: keep paging.
  const lastCursor = feed.data?.pages.at(-1)?.next_cursor ?? null;
  const searching =
    enabled &&
    rows.length === 0 &&
    !feed.isPending &&
    !feed.isError &&
    feed.hasNextPage;
  const followed = useRef<{ filter: NotificationFilter; cursors: string[] }>({
    filter,
    cursors: [],
  });
  if (followed.current.filter !== filter)
    followed.current = { filter, cursors: [] };
  const autoHop =
    searching &&
    !feed.isFetchingNextPage &&
    nextEmptyPageHop(lastCursor, followed.current.cursors);
  const fetchNextPage = feed.fetchNextPage;
  useEffect(() => {
    if (!autoHop || !runtime.active || !lastCursor) return;
    followed.current.cursors.push(lastCursor);
    void fetchNextPage();
  }, [autoHop, runtime.active, lastCursor, fetchNextPage]);
  const { refreshing, onRefresh } = usePullRefresh(() =>
    Promise.all([feed.refetch(), summary.refetch()]),
  );
  const markAll = useCallback(() => {
    if (asOf) markRead.mutate({ all: true, before: asOf });
  }, [asOf, markRead.mutate]);

  const open = useCallback(
    (item: AssistantNotification) => {
      if (!item.read) markRead.mutate({ ids: [item.id] });
      const target = notificationTarget(item, firstPartyFrontendUrls());
      switch (target.kind) {
        case "chat":
          openChat({
            conversationId: target.conversationId,
            agentId: target.agentId,
            acknowledgementId: target.acknowledgementId,
            home: false,
            draft: false,
            openLatest: target.openLatest ?? false,
          });
          return;
        case "group":
          navigation.push("GroupChat", { groupId: target.groupId });
          return;
        case "agent":
          navigation.push("AgentDetail", { agentId: target.agentId });
          return;
        case "automation":
          navigation.push("AutomationDetail", { id: target.triggerId });
          return;
        case "automations":
          navigation.push(
            "Automations",
            target.agentId ? { agentId: target.agentId } : undefined,
          );
          return;
        default:
          return;
      }
    },
    [markRead.mutate, navigation, openChat],
  );
  const segments = NOTIFICATION_FILTERS.map((value) => ({
    id: value,
    label: NOTIFICATION_FILTER_LABELS[value],
    count: notificationFilterCount(summary.data, value),
  }));
  const loadMore = () => {
    if (feed.hasNextPage && !feed.isFetchingNextPage) void feed.fetchNextPage();
  };
  let empty: React.ReactNode;
  if (!enabled) {
    empty = (
      <Notice message="Notifications are not available for this account yet." />
    );
  } else if (feed.isPending) {
    empty = (
      <SkeletonRows rows={4} accessibilityLabel="Loading notifications" />
    );
  } else if (feed.isError) {
    empty = (
      <>
        <Notice tone="error" message={notificationError(feed.error)} />
        <SettingsGroup inset={false}>
          <SettingsRow
            tone="action"
            label="Retry"
            centered
            onPress={() => void feed.refetch()}
          />
        </SettingsGroup>
      </>
    );
  } else if (searching) {
    // Not exhausted: an automatic hop is running or offered, never "all caught up".
    empty =
      autoHop || feed.isFetchingNextPage ? (
        <SkeletonRows rows={2} accessibilityLabel="Looking for updates" />
      ) : (
        <SettingsGroup inset={false}>
          <SettingsRow
            tone="action"
            label="Load more"
            centered
            onPress={() => void feed.fetchNextPage()}
          />
        </SettingsGroup>
      );
  } else {
    empty = (
      <View style={styles.empty}>
        <Text style={[styles.emptyTitle, { color: colors.textPrimary }]}>
          {NOTIFICATION_EMPTY_COPY[filter]}
        </Text>
        <Text style={[styles.emptyText, { color: colors.textMuted }]}>
          Automation results, decisions and updates from your chats appear here.
        </Text>
      </View>
    );
  }
  return (
    <WorkspacePage title="Notifications" scroll={false}>
      <SegmentControl
        segments={segments}
        activeIndex={NOTIFICATION_FILTERS.indexOf(filter)}
        accessibilityLabel="Filter notifications"
        style={styles.filters}
        onPress={(index) => setFilter(NOTIFICATION_FILTERS[index] ?? "all")}
      />
      <FlatList
        testID="notifications-list"
        data={rows}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => (
          <NotificationRow item={item} now={now} onPress={open} />
        )}
        ItemSeparatorComponent={() => (
          <View
            style={[styles.separator, { backgroundColor: colors.borderSoft }]}
          />
        )}
        ListEmptyComponent={empty}
        ListFooterComponent={
          feed.isFetchingNextPage ? (
            <ActivityIndicator
              accessibilityLabel="Loading more notifications"
              color={colors.primary}
              style={styles.footer}
            />
          ) : null
        }
        onEndReached={loadMore}
        onEndReachedThreshold={0.4}
        refreshControl={
          <RefreshControl
            accessibilityLabel="Refresh"
            refreshing={refreshing}
            onRefresh={onRefresh}
            tintColor={colors.primary}
            colors={[colors.primary]}
            progressBackgroundColor={colors.card}
          />
        }
        contentContainerStyle={styles.listContent}
        keyboardShouldPersistTaps="handled"
      />
      <View style={[styles.readFooter, { borderTopColor: colors.borderSoft }]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Mark all notifications as read"
          accessibilityState={{
            disabled: !enabled || unread === 0 || !asOf || markRead.isPending,
          }}
          disabled={!enabled || unread === 0 || !asOf || markRead.isPending}
          onPress={markAll}
          testID="notifications-mark-all"
          style={styles.footer}
        >
          <Text
            style={[
              styles.footerLabel,
              {
                color:
                  enabled && unread > 0 && asOf && !markRead.isPending
                    ? colors.primary
                    : colors.textMuted,
              },
            ]}
          >
            Mark all notifications as read
          </Text>
        </Pressable>
      </View>
    </WorkspacePage>
  );
}

function createStyles(colors: ThemeColors) {
  return StyleSheet.create({
    filters: { marginBottom: spacing.md },
    listContent: { flexGrow: 1, paddingBottom: spacing.xxl },
    separator: { height: StyleSheet.hairlineWidth, marginLeft: 56 },
    row: {
      minHeight: 64,
      flexDirection: "row",
      alignItems: "flex-start",
      gap: spacing.md,
      paddingVertical: spacing.md,
      paddingHorizontal: spacing.sm,
      borderRadius: radius.md,
    },
    iconTile: {
      width: 36,
      height: 36,
      borderRadius: radius.full,
      alignItems: "center",
      justifyContent: "center",
    },
    avatar: { position: "relative", flexShrink: 0, marginTop: 2 },
    kindBadge: {
      position: "absolute",
      bottom: -3,
      right: -3,
      width: 18,
      height: 18,
      borderRadius: 9,
      borderWidth: 2,
      alignItems: "center",
      justifyContent: "center",
    },
    emphasis: { fontFamily: fonts.bodySemi, fontWeight: "600" },
    sourceChip: {
      maxWidth: "75%",
      paddingHorizontal: 6,
      paddingVertical: 2,
      borderRadius: radius.sm,
    },
    readFooter: { borderTopWidth: StyleSheet.hairlineWidth },
    footerLabel: {
      ...typeScale.caption,
      fontFamily: fonts.bodySemi,
      textAlign: "center",
    },
    body: { flex: 1, minWidth: 0, gap: 2 },
    titleLine: {
      flexDirection: "row",
      alignItems: "baseline",
      gap: spacing.sm,
      minWidth: 0,
    },
    title: { ...typeScale.body, flex: 1, minWidth: 0 },
    titleUnread: { fontFamily: fonts.bodySemi, fontWeight: "600" as const },
    time: { ...typeScale.caption, flexShrink: 0 },
    preview: { ...typeScale.caption },
    metaLine: {
      flexDirection: "row",
      flexWrap: "wrap",
      gap: spacing.xs,
      marginTop: 2,
    },
    chip: {
      borderRadius: radius.sm,
      paddingHorizontal: 6,
      paddingVertical: 1,
    },
    chipText: { fontFamily: fonts.bodySemi, fontSize: 11, lineHeight: 15 },
    footer: { paddingVertical: spacing.lg },
    empty: {
      paddingVertical: spacing.xxl,
      paddingHorizontal: spacing.lg,
      alignItems: "center",
      gap: spacing.xs,
    },
    emptyTitle: { ...typeScale.title, textAlign: "center" },
    emptyText: {
      ...typeScale.body,
      textAlign: "center",
      color: colors.textMuted,
    },
  });
}
