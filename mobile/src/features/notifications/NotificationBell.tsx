import { Pressable, StyleSheet, View } from "react-native";
import { Bell } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { fonts, radius } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import {
  notificationBadge,
  notificationBellLabel,
} from "./notificationPresentation";

/** Header bell: unread badge capped at 9+, amber while a decision waits. */
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
  const badge = notificationBadge(unread);
  const attention = needsDecision > 0;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={notificationBellLabel(unread, needsDecision)}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      testID="notification-bell"
      style={({ pressed }) => [
        styles.button,
        { opacity: disabled ? 0.4 : pressed ? 0.65 : 1 },
      ]}
    >
      <Bell size={21} color={colors.textPrimary} />
      {badge ? (
        <View
          testID="notification-bell-badge"
          style={[
            styles.badge,
            {
              backgroundColor: attention ? colors.warning : colors.primary,
              borderColor: colors.bg,
            },
          ]}
        >
          <Text
            maxFontSizeMultiplier={1.2}
            style={[styles.badgeText, { color: colors.onPrimary }]}
          >
            {badge}
          </Text>
        </View>
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  badge: {
    position: "absolute",
    top: 6,
    right: 6,
    minWidth: 17,
    height: 17,
    paddingHorizontal: 4,
    borderRadius: 9,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: {
    fontFamily: fonts.bodySemi,
    fontSize: 10,
    lineHeight: 13,
  },
});
