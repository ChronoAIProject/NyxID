import { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

type BadgeVariant =
  | "riskHigh"
  | "riskMedium"
  | "riskLow"
  | "expiryUrgent"
  | "expiryNormal"
  | "decisionApproved"
  | "decisionDenied"
  | "decisionExpired"
  | "modeChip"
  | "orgChip";

type StatusBadgeProps = {
  variant: BadgeVariant;
  label: string;
};

function getVariantStyles(c: ThemeColors): Record<BadgeVariant, { bg: string; text: string; border: string }> {
  return {
    riskHigh: c.riskHigh,
    riskMedium: c.riskMedium,
    riskLow: c.riskLow,
    expiryUrgent: { bg: c.dangerSoftBg, text: c.danger, border: c.riskHigh.border },
    expiryNormal: { bg: c.riskLow.bg, text: c.success, border: c.riskLow.border },
    decisionApproved: { bg: c.riskLow.bg, text: c.success, border: c.riskLow.border },
    decisionDenied: { bg: c.dangerSoftBg, text: c.danger, border: c.riskHigh.border },
    decisionExpired: { bg: c.ghostBg, text: c.textMuted, border: c.borderSoft },
    modeChip: { bg: c.primaryGlow, text: c.primary, border: c.primaryGlow },
    // Org chip reuses the muted ghost palette so it reads as
    // structural context (whose decision is this?) rather than risk
    // or status. Keeps DESIGN.md tokens; no new colors introduced.
    orgChip: { bg: c.ghostBg, text: c.textSecondary, border: c.borderSoft },
  };
}

export function StatusBadge({ variant, label }: StatusBadgeProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const variantStyles = useMemo(() => getVariantStyles(colors), [colors]);

  const v = variantStyles[variant];
  return (
    <View style={[styles.badge, { backgroundColor: v.bg, borderColor: v.border }]}>
      <Text style={[styles.text, { color: v.text }]}>{label}</Text>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    badge: {
      // DESIGN.md §Layout: 100px (badges, pills) — pill-shaped
      paddingHorizontal: spacing.md,
      paddingVertical: spacing.xxs,
      borderRadius: radius.pill,
      borderWidth: 1,
      alignSelf: "flex-start",
    },
    text: {
      ...typeScale.overline,
    },
  });
