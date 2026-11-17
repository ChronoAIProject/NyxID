import { useContext, type ReactNode } from "react";
import { WorkspaceQuickRailContext } from "../../components/workspace/WorkspaceLayoutContext";
import { StyleSheet, View } from "react-native";
import { MoreHorizontal, ScanQrCode, SquarePen } from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { ChatIconButton } from "./ChatSurface";

/** Which trailing actions the chat bar carries. */
export type ChatHeaderLayout = "wide" | "narrow";

/**
 * Narrow bars: Scan QR, New chat, More. Wide bars keep the bell in Scan QR's
 * place because the persistent sidebar already offers the scanner. The Duo:
 * no bar in rail mode (the controls sit in the status column); in full
 * screen the bar carries only the column's own group (Menu, the agent,
 * More), so only More here: New chat and Scan QR stay on the rail. Targets never
 * shrink; the identity owns all width negotiation. Every bar floats each
 * control in its own 44pt glass capsule, 8pt apart.
 */
export function ChatHeaderActions({
  layout,
  scanDisabled,
  newChatDisabled,
  onScanQr,
  onNewChat,
  onOptions,
  notifications,
}: {
  layout: ChatHeaderLayout;
  scanDisabled?: boolean;
  newChatDisabled?: boolean;
  onScanQr: () => void;
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
    <View
      testID="chat-header-actions"
      style={styles.actions}
    >
      {
        notifications}
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
  actions: { flexDirection: "row", alignItems: "center", flexShrink: 0, gap: 8, },
});
