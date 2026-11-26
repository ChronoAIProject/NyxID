import { StyleSheet, View } from "react-native";
import { Bell } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { radius, typeScale } from "../../theme/designTokens";
import { FloatingButton } from "../../components/FloatingControl";
import { useTheme } from "../../theme/ThemeContext";
import {
  notificationBadge,
  notificationBellLabel,
} from "./notificationPresentation";

/**
 * Header bell: the shared floating glass button with a compact 99+ count pill
 * tinted amber for waiting decisions.
 */
export function NotificationBell({
  unread,
  needsDecision,
  onPress,
  disabled = false,
}: {
  unread: number;
  needsDecision: number;
  onPress: () => void;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  const badge = notificationBadge(unread > 0 ? unread : needsDecision);
  const attention = needsDecision > 0;
  return (
    <FloatingButton
      accessibilityLabel={notificationBellLabel(unread, needsDecision)}
      disabled={disabled}
      onPress={onPress}
      testID="notification-bell"
      style={styles.button}
    >
      <View testID="notification-bell-glyph" style={styles.glyph}>
        <Bell size={21} color={colors.textPrimary} />
      </View>
      {badge ? (
        <View
          pointerEvents="none"
          testID="notification-bell-badge"
          style={[styles.badge, { backgroundColor: attention ? colors.warning : colors.primary }]}
        >
          <Text
            maxFontSizeMultiplier={1.2}
            style={[styles.badgeText, { color: attention ? colors.onWarning : colors.onPrimary }]}
          >
            {badge}
          </Text>
        </View>
      ) : null}
    </FloatingButton>
  );
}

const styles = StyleSheet.create({
  button: { flexDirection: "row", gap: 4, paddingHorizontal: 8 },
  glyph: { width: 21, height: 21 },
  badge: {
    minWidth: 18,
    height: 18,
    paddingHorizontal: 4,
    borderRadius: radius.full,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: {
    ...typeScale.countBadge,
    fontVariant: ["tabular-nums"],
  },
});
