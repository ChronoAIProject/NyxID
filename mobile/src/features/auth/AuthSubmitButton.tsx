import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { fonts } from "../../theme/designTokens";
import { useAuthTheme } from "./useAuthTheme";

/** All auth submits retain their primary identity, including empty and pending forms. */
export function AuthSubmitButton({
  label,
  onPress,
  disabled = false,
  loading = false,
  style,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  loading?: boolean;
  style?: StyleProp<ViewStyle>;
}) {
  const { colors, mode } = useAuthTheme();
  const blocked = disabled || loading;
  const dimmed = disabled && !loading;
  const foreground =
    dimmed && mode === "light" ? colors.textPrimary : colors.onPrimary;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: blocked, busy: loading }}
      disabled={blocked}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        {
          backgroundColor: colors.bg,
          shadowColor: colors.primary,
          shadowOpacity: dimmed ? 0 : 0.3,
        },
        style,
        pressed && !blocked && styles.pressed,
      ]}
    >
      {({ pressed }) => (
        <>
          <LinearGradient
            pointerEvents="none"
            colors={
              pressed && !blocked
                ? [colors.gradientStartPressed, colors.gradientEndPressed]
                : [colors.gradientStart, colors.gradientEnd]
            }
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            style={[StyleSheet.absoluteFillObject, dimmed && styles.dimmed]}
          />
          <View
            pointerEvents="none"
            style={[
              StyleSheet.absoluteFillObject,
              styles.edge,
              { borderColor: dimmed ? colors.primary : "transparent" },
            ]}
          />
          <View style={styles.content}>
            {loading && (
              <ActivityIndicator
                accessible={false}
                size="small"
                color={foreground}
              />
            )}
            <Text style={[styles.label, { color: foreground }]}>{label}</Text>
          </View>
        </>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: {
    borderRadius: 14,
    overflow: "hidden",
    minHeight: 52,
    justifyContent: "center",
    shadowOffset: { width: 0, height: 4 },
    shadowRadius: 12,
    elevation: 6,
  },
  content: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
    paddingHorizontal: 20,
    paddingVertical: 16,
  },
  label: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 20 },
  dimmed: { opacity: 0.5 },
  edge: { borderRadius: 14, borderWidth: 1 },
  pressed: { transform: [{ scale: 0.98 }] },
});
