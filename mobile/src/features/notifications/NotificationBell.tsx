import { Pressable, StyleSheet, View } from "react-native";
import { Bell } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { radius, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import {
  notificationBadge,
  notificationBellLabel,
} from "./notificationPresentation";

/** Header bell: a compact 99+ count pill and a separate decision marker. */
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
      <View testID="notification-bell-glyph" style={styles.glyph}>
        <Bell size={21} color={colors.textPrimary} />
        {badge ? (
          <View
            testID="notification-bell-badge"
            style={[
              styles.badge,
              {
                backgroundColor: colors.card,
                borderColor: colors.bg,
              },
            ]}
          >
            <Text
              maxFontSizeMultiplier={1.2}
              style={[styles.badgeText, { color: colors.textPrimary }]}
            >
              {badge}
            </Text>
          </View>
        ) : null}
        {attention ? <View testID="notification-bell-attention" style={[styles.attention, { backgroundColor: colors.warning, borderColor: colors.bg }]} /> : null}
      </View>
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
  glyph: { position: "relative", width: 21, height: 21 },
  badge: {
    position: "absolute",
    top: -6,
    right: -8,
    minWidth: 18,
    height: 18,
    paddingHorizontal: 4,
    borderRadius: 9,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: {
    ...typeScale.badge,
    fontVariant: ["tabular-nums"],
  },
  attention: { position: "absolute", bottom: -2, right: -2, width: 8, height: 8, borderRadius: radius.full, borderWidth: 1 },
});
