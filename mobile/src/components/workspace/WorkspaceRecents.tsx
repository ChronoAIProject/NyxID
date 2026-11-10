import { Pressable, StyleSheet, Text, View } from "react-native";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { fonts } from "../../theme/designTokens";
import { SkeletonRows } from "../Skeleton";
import { useTheme } from "../../theme/ThemeContext";
import type { WorkspaceRecent } from "./workspaceSidebarModel";
import { useWorkspaceRecents } from "./useWorkspaceRecents";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import {
  EMPTY_NOTIFICATION_MARKERS,
  type NotificationMarkers,
} from "../../features/notifications/notificationPresentation";

/** Unread notifications attributed to a recent thread or crew. */
export function recentUnread(
  item: Pick<WorkspaceRecent, "target">,
  markers: NotificationMarkers,
): number {
  const target = item.target;
  if (target.kind === "group") return markers.groups[target.groupId] ?? 0;
  if (target.kind === "agent" && target.conversationId)
    return markers.conversations[target.conversationId] ?? 0;
  return 0;
}

export function WorkspaceRecentRow({
  item,
  unread = 0,
  onSelect,
}: {
  item: WorkspaceRecent;
  unread?: number;
  onSelect: (target: WorkspaceChatTarget) => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open recent ${item.title}${item.status ? `, ${item.status}` : ""}${unread > 0 ? `, ${unread} unread` : ""}`}
      onPress={() => onSelect(item.target)}
      style={({ pressed }) => [
        styles.row,
        { backgroundColor: pressed ? colors.navActive : "transparent" },
      ]}
    >
      <Text
        numberOfLines={1}
        style={[
          styles.title,
          { color: colors.textPrimary },
          unread > 0 ? styles.titleUnread : null,
        ]}
      >
        {item.title}
      </Text>
      {unread > 0 ? (
        <Text
          testID="sidebar-unread-count"
          style={[
            styles.count,
            { color: colors.textPrimary, backgroundColor: colors.navActive },
          ]}
        >
          {unread > 99 ? "99+" : String(unread)}
        </Text>
      ) : null}
      {item.status ? (
        <Text
          numberOfLines={1}
          style={[
            styles.status,
            {
              color:
                item.status === "Needs you"
                  ? colors.warningTone.text
                  : colors.textMuted,
            },
          ]}
        >
          {item.status}
        </Text>
      ) : null}
      <Text style={[styles.time, { color: colors.textMuted }]}>
        {formatRelativeTimeFromMs(Date.parse(item.activityAt))}
      </Text>
    </Pressable>
  );
}
export function WorkspaceRecents({
  visible,
  unread = EMPTY_NOTIFICATION_MARKERS,
  onSelect,
  onSeeAll,
}: {
  visible: boolean;
  /** Per-thread unread counts from the sidebar's one notification summary. */
  unread?: NotificationMarkers;
  onSelect: (target: WorkspaceChatTarget) => void;
  onSeeAll: () => void;
}) {
  const { colors } = useTheme();
  const data = useWorkspaceRecents(visible);
  return (
    <View testID="sidebar-recents">
      <Text
        accessibilityRole="header"
        style={[styles.heading, { color: colors.textPrimary }]}
      >
        Recents
      </Text>
      {data.rows.slice(0, 8).map((item) => (
        <WorkspaceRecentRow
          key={item.key}
          item={item}
          unread={recentUnread(item, unread)}
          onSelect={onSelect}
        />
      ))}
      {!data.rows.length && data.online && !data.error && data.loading ? (
        <SkeletonRows
          rows={3}
          icon={false}
          detail={false}
          accessibilityLabel="Loading recent chats"
        />
      ) : !data.rows.length ? (
        <Text style={[styles.notice, { color: colors.textMuted }]}>
          {!data.online
            ? "Offline · showing saved recent chats."
            : data.error
              ? "Couldn’t load recent chats."
              : "No recent chats."}
        </Text>
      ) : null}
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="See all recent chats"
        onPress={onSeeAll}
        style={styles.row}
      >
        <Text style={[styles.title, { color: colors.textMuted }]}>
          See all…
        </Text>
      </Pressable>
    </View>
  );
}
const styles = StyleSheet.create({
  heading: {
    ...sidebarSectionTitle,
    paddingHorizontal: 12,
    paddingTop: 22,
    paddingBottom: 10,
  },
  row: {
    minHeight: 44,
    paddingHorizontal: 12,
    paddingVertical: 10,
    flexDirection: "row",
    gap: 8,
    alignItems: "center",
    borderRadius: 8,
  },
  title: { flex: 1, fontFamily: fonts.bodyRegular, fontSize: 14 },
  titleUnread: { fontFamily: fonts.bodySemi },
  count: {
    fontFamily: fonts.bodySemi,
    fontSize: 11,
    borderRadius: 10,
    paddingHorizontal: 7,
    paddingVertical: 2,
    overflow: "hidden",
  },
  status: { maxWidth: 78, fontFamily: fonts.bodyRegular, fontSize: 10 },
  time: { fontFamily: fonts.bodyRegular, fontSize: 10 },
  notice: { fontFamily: fonts.bodyRegular, fontSize: 12, padding: 12 },
});
