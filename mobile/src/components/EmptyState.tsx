import { useMemo, type ReactElement } from "react";
import { StyleSheet, Text, View } from "react-native";
import { spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { BiometricLockIllustration } from "./icons/empty-state/BiometricLockIllustration";
import { SmartLockIllustration } from "./icons/empty-state/SmartLockIllustration";
import { SeoKeywordIllustration } from "./icons/empty-state/SeoKeywordIllustration";

type EmptyStatePreset = "pendingEmpty" | "activeEmpty" | "historyEmpty";

type EmptyStateProps = {
  preset: EmptyStatePreset;
};

type PresetConfig = {
  Illustration: (props: { size?: number; color: string }) => ReactElement;
  title: string;
  subtitle: string;
};

// DESIGN.md §Empty state: rich SVG illustrations from `components/icons/empty-state/`,
// "Never use a generic Lucide icon at this size". Three distinct lock/auth-themed
// illustrations from the web's empty-state library — each tab gets its own.
//
//   - pendingEmpty  →  BiometricLockIllustration (web uses on pages/consents.tsx —
//                      auth-themed; reads as "lock waiting on a challenge")
//   - activeEmpty   →  SmartLockIllustration     (web 1:1 from pages/approval-grants.tsx)
//   - historyEmpty  →  SeoKeywordIllustration    (web 1:1 from pages/approval-history.tsx)
const presetConfigs: Record<EmptyStatePreset, PresetConfig> = {
  pendingEmpty: {
    Illustration: BiometricLockIllustration,
    title: "No pending challenges",
    subtitle: "New high-risk requests will appear here.",
  },
  activeEmpty: {
    Illustration: SmartLockIllustration,
    title: "No active approvals",
    subtitle: "Approvals appear here after challenge decisions.",
  },
  historyEmpty: {
    Illustration: SeoKeywordIllustration,
    title: "No decision history",
    subtitle: "Past approvals, denials, and expirations appear here.",
  },
};

export function EmptyState({ preset }: EmptyStateProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const { Illustration, title, subtitle } = presetConfigs[preset];

  // DESIGN.md §Empty state recipe: centered column, illustration at 30% opacity
  // muted-foreground, 12px font-medium muted headline, 12px supporting line.
  // Mobile bumps the headline to 13px semibold for legibility without breaking
  // the muted tone discipline.
  return (
    <View style={styles.container}>
      <Illustration size={192} color={colors.textTertiary} />
      <Text style={styles.title}>{title}</Text>
      <Text style={styles.subtitle}>{subtitle}</Text>
    </View>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  container: {
    paddingVertical: spacing.huge + spacing.md,
    paddingHorizontal: spacing.xxl,
    alignItems: "center",
    gap: spacing.xs,
  },
  title: {
    ...typeScale.bodyStrong,
    color: c.textMuted,
    textAlign: "center",
    marginTop: spacing.sm,
  },
  subtitle: {
    ...typeScale.small,
    color: c.textTertiary,
    textAlign: "center",
  },
});
