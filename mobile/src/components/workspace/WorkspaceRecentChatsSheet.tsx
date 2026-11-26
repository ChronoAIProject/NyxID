import { StyleSheet, View } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { useWorkspaceSheet } from "../../app/WorkspaceHeader";
import { useNotificationIndicator } from "../../features/notifications/useNotifications";
import { SkeletonRows } from "../Skeleton";
import { WorkspacePage } from "./WorkspaceUI";
import { WorkspaceRecentRow } from "./WorkspaceRecents";
import { useWorkspaceRecents } from "./useWorkspaceRecents";
import { recentUnread } from "./workspaceSidebarModel";

/** The quick rail's Recent chats: the sidebar's Recents rows in the shared sheet. */
export function WorkspaceRecentChatsSheet() {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const sheet = useWorkspaceSheet();
  const focused = useIsFocused();
  const data = useWorkspaceRecents(focused);
  const notifications = useNotificationIndicator(data.active);
  const empty = !data.rows.length;
  return (
    <WorkspacePage title={t("Recent chats")} showHeading={false}>
      <View testID="recent-chats-sheet" style={styles.list}>
        {data.rows.map((item) => (
          <WorkspaceRecentRow
            key={item.key}
            item={item}
            unread={recentUnread(item, notifications.markers)}
            // Like the sidebar: the sheet closes and that thread opens.
            onSelect={(target) => sheet?.onOpenTarget(target)}
          />
        ))}
        {empty && data.online && !data.error && data.loading ? (
          <SkeletonRows
            rows={3}
            icon={false}
            detail={false}
            accessibilityLabel={t("Loading recent chats")}
          />
        ) : empty ? (
          <Text style={[styles.notice, { color: colors.textMuted }]}>
            {!data.online
              ? t("Offline · showing saved recent chats.")
              : data.error
                ? t("Couldn’t load recent chats.")
                : t("No recent chats.")}
          </Text>
        ) : null}
      </View>
    </WorkspacePage>
  );
}

const styles = StyleSheet.create({
  list: { gap: 2 },
  notice: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 20,
    paddingVertical: 12,
  },
});
