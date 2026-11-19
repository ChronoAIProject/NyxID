import { useContext, type ReactNode } from "react";
import { WorkspaceQuickRailContext } from "../../components/workspace/WorkspaceLayoutContext";
import { StyleSheet, View } from "react-native";
import { MoreHorizontal, SquarePen } from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { ChatIconButton } from "./ChatSurface";

/**
 * Ordinary bars: the bell (when notifications are enabled), New chat and More.
 * Duo rail mode renders only More here; New chat and Scan QR are on the rail.
 * Scan QR is otherwise in the side menu's Quick actions. Targets never shrink;
 * the identity owns all width negotiation. Every bar floats each control in
 * its own 44pt glass capsule, 8pt apart.
 */
export function ChatHeaderActions({
  newChatDisabled,
  onNewChat,
  onOptions,
  notifications,
}: {
  newChatDisabled?: boolean;
  onNewChat: () => void;
  onOptions: () => void;
  notifications?: ReactNode;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const duo = useContext(WorkspaceQuickRailContext);
  const more = (
    <ChatIconButton
      label={t("Chat options")}
      onPress={onOptions}
      floating
    >
      <MoreHorizontal size={23} color={colors.textPrimary} />
    </ChatIconButton>
  );
  if (duo)
    return (
      <View testID="chat-header-actions" style={styles.actions}>
        {more}
      </View>
    );
  return (
    <View testID="chat-header-actions" style={styles.actions}>
      {notifications}
      <ChatIconButton
        label={t("New chat")}
        onPress={onNewChat}
        disabled={newChatDisabled}
        floating
      >
        <SquarePen size={21} color={colors.textPrimary} />
      </ChatIconButton>
      {more}
    </View>
  );
}

const styles = StyleSheet.create({
  actions: { flexDirection: "row", alignItems: "center", flexShrink: 0, gap: 8 },
});
