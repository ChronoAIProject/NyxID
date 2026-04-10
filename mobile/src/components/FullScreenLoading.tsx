import { useMemo } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";
import { radius, spacing } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

import { ScreenContainer } from "./ScreenContainer";

type FullScreenLoadingProps = {
  title?: string;
  subtitle?: string;
};

export function FullScreenLoading({
  title = "Loading...",
  subtitle = "Syncing the latest data, please wait.",
}: FullScreenLoadingProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  return (
    <ScreenContainer>
      <View style={styles.center}>
        <View style={styles.card}>
          <ActivityIndicator size="small" color={colors.primary} />
          <Text style={styles.title}>{title}</Text>
          <Text style={styles.subtitle}>{subtitle}</Text>
        </View>
      </View>
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    center: {
      flex: 1,
      justifyContent: "center",
      alignItems: "center",
      paddingHorizontal: spacing.xxl,
      paddingBottom: 72,
    },
    card: {
      width: "100%",
      borderRadius: radius.lg,
      borderWidth: 1,
      borderColor: c.border,
      backgroundColor: c.card,
      paddingVertical: spacing.xxl,
      paddingHorizontal: spacing.xl,
      alignItems: "center",
      gap: spacing.sm,
    },
    title: {
      color: c.textPrimary,
      fontSize: 16,
      fontWeight: "600",
    },
    subtitle: {
      color: c.textMuted,
      fontSize: 13,
      textAlign: "center",
    },
  });
