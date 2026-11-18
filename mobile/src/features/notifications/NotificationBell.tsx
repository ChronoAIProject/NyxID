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
    >
      <View testID="notification-bell-glyph" style={styles.glyph}>
        <Bell size={21} color={colors.textPrimary} />
        {badge ? (
          <View
            testID="notification-bell-badge-ring"
            style={[styles.badgeRing, { borderColor: colors.sidebarBase }]}
          >
            <View
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
          </View>
        ) : null}
      </View>
    </FloatingButton>
  );
}

const styles = StyleSheet.create({
  glyph: { position: "relative", width: 21, height: 21 },
  badgeRing: {
    position: "absolute",
    // The fill starts at the 21px glyph's midpoint; the ring sits outside it.
    left: 10.5 - 2,
    top: -6 - 2,
    borderWidth: 2,
    borderRadius: radius.full,
  },
  badge: {
    minWidth: 14,
    height: 14,
    paddingHorizontal: 3,
    borderRadius: radius.full,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: {
    ...typeScale.countBadge,
    fontVariant: ["tabular-nums"],
  },
});
