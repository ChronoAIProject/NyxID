import { useContext } from "react";
import { WorkspaceCornerSensorContext } from "./WorkspaceLayoutContext";
import { workspaceRecentLabel } from "./workspaceAccessibilityLabels";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import {
  Text, Pressable, StyleSheet, View, useWindowDimensions, } from "react-native";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { fonts } from "../../theme/designTokens";
import { SkeletonRows } from "../Skeleton";
import { useTheme } from "../../theme/ThemeContext";
import { recentUnread, type WorkspaceRecent } from "./workspaceSidebarModel";
import { useWorkspaceRecents } from "./useWorkspaceRecents";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import {
  EMPTY_NOTIFICATION_MARKERS,
  type NotificationMarkers,
} from "../../features/notifications/notificationPresentation";

/**
 * iOS accessibility text sizes start above XXXL (RN fontScale 1.353; AX1 is
 * 1.786). From there the row becomes a column: the title takes the whole
 * first line and the unread count, status and time move beneath it, so a long
 * relative time can never squeeze the title down to a few letters. Every
 * label stays one line.
 */
export const RECENT_ROW_STACK_FONT_SCALE = 1.5;

export function WorkspaceRecentRow({
  item,
  unread = 0,
  disabled = false,
  onSelect,
}: {
  item: WorkspaceRecent;
  unread?: number;
  disabled?: boolean;
  onSelect: (target: WorkspaceChatTarget) => void;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const stacked =
    useContext(WorkspaceCornerSensorContext) && fontScale >= RECENT_ROW_STACK_FONT_SCALE;
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled}
      accessibilityState={{ disabled }}
      accessibilityLabel={workspaceRecentLabel(item.title, item.status, unread)}
      onPress={() => onSelect(item.target)}
      style={({ pressed }) => [
        styles.row,
        stacked ? styles.rowStacked : null,
        { backgroundColor: pressed ? colors.navActive : "transparent" },
      ]}
    >
      <Text
        numberOfLines={1}
        style={[
          styles.title,
          stacked ? styles.titleStacked : null,
          { color: disabled ? colors.textMuted : colors.textPrimary },
          unread > 0 ? styles.titleUnread : null,
        ]}
      >
        {item.title}
      </Text>
      <View
        testID="recent-row-meta"
        style={[styles.meta, stacked ? styles.metaStacked : null]}
      >
        {unread > 0 ? (
          <Text
            testID="sidebar-unread-count"
            style={[
              styles.count,
              {
                color: disabled ? colors.textMuted : colors.textPrimary,
                backgroundColor: colors.navActive,
              },
            ]}
          >
            {unread > 99 ? "99+" : String(unread)}
          </Text>
        ) : null}
        {item.status ? (
          <Text
            style={[
              styles.status,
              stacked ? styles.statusStacked : null,
              {
                color:
                  !disabled && item.status === "Needs you"
                    ? colors.warningTone.text
                    : colors.textMuted,
              },
            ]}
          >
            {translateApp(item.status)}
          </Text>
        ) : null}
        <Text
          numberOfLines={1}
          style={[styles.time, { color: colors.textMuted }]}
        >
          {formatRelativeTimeFromMs(Date.parse(item.activityAt))}
        </Text>
      </View>
    </Pressable>
  );
}
export function WorkspaceRecents({
  visible,
  disabled = false,
  unread = EMPTY_NOTIFICATION_MARKERS,
  onSelect,
  onSeeAll,
}: {
  visible: boolean;
  disabled?: boolean;
  /** Per-thread unread counts from the sidebar's one notification summary. */
  unread?: NotificationMarkers;
  onSelect: (target: WorkspaceChatTarget) => void;
  onSeeAll: () => void;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const data = useWorkspaceRecents(visible);
  return (
    <View testID="sidebar-recents">
      <Text
        accessibilityRole="header"
        style={[styles.heading, { color: colors.textPrimary }]}
      >
        {translateApp("Recents")}
      </Text>
      {data.rows.slice(0, 8).map((item) => (
        <WorkspaceRecentRow
          key={item.key}
          item={item}
          disabled={disabled}
          unread={recentUnread(item, unread)}
          onSelect={onSelect}
        />
      ))}
      {!data.rows.length && data.online && !data.error && data.loading ? (
        <SkeletonRows
          rows={3}
          icon={false}
          detail={false}
          accessibilityLabel={translateApp("Loading recent chats")}
        />
      ) : !data.rows.length ? (
        <Text
          style={[styles.notice, { color: colors.textMuted }]}
        >
          {!data.online
            ? translateApp("Offline · showing saved recent chats.")
            : data.error
              ? translateApp("Couldn’t load recent chats.")
              : translateApp("No recent chats.")}
        </Text>
      ) : null}
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={translateApp("See all recent chats")}
        disabled={disabled}
        accessibilityState={{ disabled }}
        onPress={onSeeAll}
        style={styles.row}
      >
        <Text
          style={[styles.title, { color: colors.textMuted }]}
        >
          {translateApp("See all…")}
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
  // Accessibility sizes: the title takes the row's full width on its own
  // line (still one line, ellipsized) and the meta moves beneath it.
  rowStacked: { flexDirection: "column", alignItems: "stretch", gap: 2 },
  title: { flex: 1, fontFamily: fonts.bodyRegular, fontSize: 14 },
  titleStacked: { flex: 0 },
  // Count, status and time: beside the title, or beneath it when stacked.
  meta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minWidth: 0,
    flexShrink: 1,
  },
  metaStacked: { flexWrap: "wrap", rowGap: 2 },
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
  statusStacked: { maxWidth: "100%" },
  time: {
    minWidth: 0,
    flexShrink: 1,
    fontFamily: fonts.bodyRegular,
    fontSize: 10,
  },
  notice: { fontFamily: fonts.bodyRegular, fontSize: 12, padding: 12 },
});
