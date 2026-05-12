import { StyleSheet } from "react-native";
import type { ThemeColors } from "./mobileTheme";
import { radius, spacing, typeScale } from "./designTokens";

export function createFlowStyles(colors: ThemeColors) {
  return StyleSheet.create({
    content: {
      flex: 1,
    },
    scrollContent: {
      paddingTop: spacing.sm,
      gap: spacing.huge,
      paddingBottom: spacing.huge,
    },
    title: {
      ...typeScale.h1,
      color: colors.textPrimary,
    },
    subtitle: {
      ...typeScale.description,
      color: colors.textSecondary,
    },
    // DESIGN.md §Layout: 10px (cards, panels) — uses radius.lg.
    card: {
      borderRadius: radius.lg,
      borderWidth: 1,
      borderColor: colors.border,
      backgroundColor: colors.card,
      padding: spacing.xl,
      gap: spacing.md,
    },
    cardTitle: {
      ...typeScale.title,
      color: colors.textPrimary,
    },
    // Detail row dividers use the subtler `borderSoft` per DESIGN.md.
    row: {
      borderBottomWidth: 1,
      borderBottomColor: colors.borderSoft,
      paddingVertical: spacing.md,
      flexDirection: "row",
      justifyContent: "space-between",
      alignItems: "center",
      gap: spacing.sm,
    },
    rowLast: {
      paddingTop: spacing.md,
      flexDirection: "row",
      justifyContent: "space-between",
      alignItems: "center",
      gap: spacing.sm,
    },
    rowLabel: {
      ...typeScale.body,
      color: colors.textMuted,
    },
    rowValue: {
      ...typeScale.label,
      color: colors.textPrimary,
      flexShrink: 1,
      textAlign: "right",
    },
    actionWrap: {
      gap: spacing.md,
    },
  });
}
