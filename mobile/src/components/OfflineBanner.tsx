import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { Feather } from "@expo/vector-icons";
import { radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

type OfflineBannerProps = {
  subtitle?: string;
  onRetry?: () => void;
};

export function OfflineBanner({ subtitle = "Showing cached data", onRetry }: OfflineBannerProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  return (
    <View style={styles.banner}>
      <Feather name="wifi-off" size={16} color={colors.dangerSoft} />
      <View style={styles.textWrap}>
        <Text style={styles.title}>No connection</Text>
        <Text style={styles.subtitle}>{subtitle}</Text>
      </View>
      {onRetry ? (
        <Pressable style={styles.retryBtn} onPress={onRetry}>
          <Text style={styles.retryText}>Retry</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    banner: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.sm,
      paddingHorizontal: spacing.lg,
      paddingVertical: spacing.sm,
      borderRadius: radius.md,
      backgroundColor: c.dangerSoftBg,
      borderWidth: 1,
      borderColor: c.danger,
      marginBottom: spacing.lg,
    },
    textWrap: {
      flex: 1,
    },
    title: {
      ...typeScale.label,
      color: c.danger,
    },
    subtitle: {
      ...typeScale.overline,
      color: c.textMuted,
      letterSpacing: 0,
      textTransform: "none",
      marginTop: 1,
    },
    retryBtn: {
      paddingHorizontal: spacing.md,
      paddingVertical: spacing.xs,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.danger,
      backgroundColor: c.dangerSoftBg,
    },
    retryText: {
      ...typeScale.overline,
      color: c.danger,
    },
  });
