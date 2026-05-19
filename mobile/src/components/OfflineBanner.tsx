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
      <View style={styles.iconTile}>
        <Feather name="wifi-off" size={16} color={colors.dangerTone.text} />
      </View>
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
    // DESIGN.md §Banners & callouts: rounded-xl border border-{color}/15 bg-{color}/[0.04],
    // 36×36 icon tile, message text-[12px] text-{color}. Destructive variant —
    // theme-aware via `dangerTone`, so both light + dark render with the right hex.
    banner: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.lg,
      paddingHorizontal: spacing.xxl,
      paddingVertical: spacing.lg,
      borderRadius: radius.lg,
      backgroundColor: c.dangerTone.bg,
      borderWidth: 1,
      borderColor: c.dangerTone.border,
      marginBottom: spacing.lg,
    },
    iconTile: {
      width: 36,
      height: 36,
      borderRadius: radius.md,
      backgroundColor: c.dangerTone.bg,
      alignItems: "center",
      justifyContent: "center",
    },
    textWrap: {
      flex: 1,
    },
    title: {
      ...typeScale.label,
      color: c.dangerTone.text,
    },
    subtitle: {
      ...typeScale.small,
      color: c.textMuted,
      letterSpacing: 0,
      textTransform: "none",
      marginTop: 1,
    },
    retryBtn: {
      paddingHorizontal: spacing.lg,
      paddingVertical: spacing.xs + spacing.xxs,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.dangerTone.border,
      backgroundColor: c.dangerTone.bg,
    },
    retryText: {
      ...typeScale.label,
      color: c.dangerTone.text,
    },
  });
