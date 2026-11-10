import { useCallback, useMemo, useState } from "react";
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
  CircleAlert,
  CircleHelp,
  MessageSquare,
  type LucideIcon,
} from "lucide-react-native";
import type { WorkspaceScreenProps } from "../../app/navigationTypes";
import { useOpenWorkspaceChat, useWorkspaceBack } from "../../app/WorkspaceHeader";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { SegmentControl } from "../../components/SegmentControl";
import { SkeletonRows } from "../../components/Skeleton";
import {
  SettingsGroup,
  SettingsRow,
  useSettingsHeaderAction,
} from "../../components/settings/SettingsList";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { usePullRefresh } from "../../hooks/usePullRefresh";
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
  automation_result: CalendarClock,
  automation_problem: CircleAlert,
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
    case "info":
      return colors.infoTone;
    default:
      return colors.primaryTone;
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
        { backgroundColor: pressed ? colors.navActive : "transparent" },
      ]}
    >
      <View style={[styles.iconTile, { backgroundColor: palette.bg }]}>
        <Icon size={18} color={palette.text} />
      </View>
      <View style={styles.body}>
        <View style={styles.titleLine}>
          <Text
            numberOfLines={1}
            style={[
              styles.title,
              { color: colors.textPrimary },
              !item.read && styles.titleUnread,
            ]}
          >
            {notificationTitle(item)}
          </Text>
          {time ? (
            <Text style={[styles.time, { color: colors.textMuted }]}>{time}</Text>
          ) : null}
        </View>
        {item.preview ? (
          <Text
            numberOfLines={2}
            style={[styles.preview, { color: colors.textSecondary }]}
          >
            {item.preview}
          </Text>
        ) : null}
        {count || pending ? (
          <View style={styles.metaLine}>
            {count ? (
              <Chip label={count} tone="default" colors={colors} styles={styles} />
            ) : null}
            {pending ? (
              <Chip label="Needs you" tone="warning" colors={colors} styles={styles} />
            ) : null}
          </View>
        ) : null}
      </View>
      {!item.read ? (
        <View
          accessibilityLabel="Unread"
          testID="notification-unread-dot"
          style={[
            styles.dot,
            { backgroundColor: pending ? colors.warning : colors.primary },
          ]}
        />
      ) : null}
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
  const { refreshing, onRefresh } = usePullRefresh(() =>
    Promise.all([feed.refetch(), summary.refetch()]),
  );
  const markAll = useCallback(() => {
    if (asOf) markRead.mutate({ all: true, before: asOf });
  }, [asOf, markRead.mutate]);
  useSettingsHeaderAction(navigation, {
    label: "Mark all read",
    enabled: enabled && unread > 0 && !!asOf && !markRead.isPending,
    busy: markRead.isPending,
    onPress: markAll,
  });
  const open = useCallback(
    (item: AssistantNotification) => {
      if (!item.read) markRead.mutate({ ids: [item.id] });
      const target = notificationTarget(item);
      switch (target.kind) {
        case "chat":
          openChat({
            conversationId: target.conversationId,
            agentId: target.agentId,
            home: false,
            draft: false,
            openLatest: false,
          });
          return;
        case "group":
          navigation.push("GroupChat", { groupId: target.groupId });
          return;
        case "automation":
          navigation.push("AutomationDetail", { id: target.triggerId });
          return;
        case "automations":
          navigation.push("Automations");
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
    ...(value === "needs_you" && summary.data?.needs_decision_count
      ? { count: summary.data.needs_decision_count }
      : {}),
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
    empty = <SkeletonRows rows={4} accessibilityLabel="Loading notifications" />;
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
          <View style={[styles.separator, { backgroundColor: colors.borderSoft }]} />
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
      borderRadius: radius.md,
      alignItems: "center",
      justifyContent: "center",
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
    dot: { width: 8, height: 8, borderRadius: 4, marginTop: 6 },
    footer: { paddingVertical: spacing.lg },
    empty: {
      paddingVertical: spacing.xxl,
      paddingHorizontal: spacing.lg,
      alignItems: "center",
      gap: spacing.xs,
    },
    emptyTitle: { ...typeScale.title, textAlign: "center" },
    emptyText: { ...typeScale.body, textAlign: "center", color: colors.textMuted },
  });
}
