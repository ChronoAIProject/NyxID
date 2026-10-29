import { Pressable, StyleSheet, Text, View } from "react-native";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { WorkspaceRecent } from "./workspaceSidebarModel";
import { useWorkspaceRecents } from "./useWorkspaceRecents";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";

export function WorkspaceRecentRow({
  item,
  onSelect,
}: {
  item: WorkspaceRecent;
  onSelect: (target: WorkspaceChatTarget) => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open recent ${item.title}${item.status ? `, ${item.status}` : ""}`}
      onPress={() => onSelect(item.target)}
      style={({ pressed }) => [
        styles.row,
        { backgroundColor: pressed ? colors.navActive : "transparent" },
      ]}
    >
      <Text
        numberOfLines={1}
        style={[styles.title, { color: colors.textPrimary }]}
      >
        {item.title}
      </Text>
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
  onSelect,
  onSeeAll,
}: {
  visible: boolean;
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
        <WorkspaceRecentRow key={item.key} item={item} onSelect={onSelect} />
      ))}
      {!data.rows.length ? (
        <Text style={[styles.notice, { color: colors.textMuted }]}>
          {!data.online
            ? "Offline · showing saved recent chats."
            : data.error
              ? "Couldn’t load recent chats."
              : data.loading
                ? "Loading recent chats…"
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
  status: { maxWidth: 78, fontFamily: fonts.bodyRegular, fontSize: 10 },
  time: { fontFamily: fonts.bodyRegular, fontSize: 10 },
  notice: { fontFamily: fonts.bodyRegular, fontSize: 12, padding: 12 },
});
