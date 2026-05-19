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

// DESIGN.md §Badges: `border-{color}/30 bg-{color}/15 text-{color}` (success/warning/info use `/10` fill).
// All variants flow through theme tokens so light + dark both render with the right hex.
function getVariantStyles(c: ThemeColors): Record<BadgeVariant, { bg: string; text: string; border: string }> {
  return {
    riskHigh: c.riskHigh,
    riskMedium: c.riskMedium,
    riskLow: c.riskLow,
    expiryUrgent: c.dangerTone,
    expiryNormal: c.successTone,
    decisionApproved: c.successTone,
    decisionDenied: c.dangerTone,
    decisionExpired: { bg: c.ghostBg, text: c.textMuted, border: c.borderSoft },
    // modeChip carries identity (purple, soft fill) per DESIGN.md `default`/`accent` badge variant.
    modeChip: { bg: c.primaryTone.bg, text: c.primaryOnTint, border: c.primaryTone.border },
    // Org chip reuses the muted ghost palette so it reads as structural context.
    orgChip: { bg: c.ghostBg, text: c.textSecondary, border: c.borderSoft },
  };
}

/**
 * DESIGN.md §Interaction Rules: "Status badges: Always title-case
 * (`Active`, `Pending`, not `active`, `PENDING`)". This helper normalizes
 * caller-provided labels so the contract holds regardless of source.
 * Hyphenated segments collapse to a single capital ("Per-request" stays
 * as-is rather than becoming "Per-Request").
 */
function toTitleCase(label: string): string {
  return label
    .split(" ")
    .map((word) => {
      if (!word) return word;
      // Preserve separator-prefixed tokens (e.g. "·") verbatim.
      if (!/[a-z]/i.test(word)) return word;
      return word.charAt(0).toUpperCase() + word.slice(1).toLowerCase();
    })
    .join(" ");
}

export function StatusBadge({ variant, label }: StatusBadgeProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const variantStyles = useMemo(() => getVariantStyles(colors), [colors]);

  const v = variantStyles[variant];
  return (
    <View style={[styles.badge, { backgroundColor: v.bg, borderColor: v.border }]}>
      <Text style={[styles.text, { color: v.text }]}>{toTitleCase(label)}</Text>
    </View>
  );
}

const createStyles = (_c: ThemeColors) =>
  StyleSheet.create({
    badge: {
      // DESIGN.md §Badges: padding px-2 py-0.5, rounded-md (6px), 10px font-medium.
      paddingHorizontal: spacing.sm,
      paddingVertical: 2,
      borderRadius: radius.sm,
      borderWidth: 1,
      alignSelf: "flex-start",
    },
    text: {
      ...typeScale.badge,
    },
  });
