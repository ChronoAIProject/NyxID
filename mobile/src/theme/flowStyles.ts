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
      ...typeScale.pageHeader,
      color: colors.textPrimary,
    },
    subtitle: {
      ...typeScale.label,
      color: colors.textSecondary,
    },
    // DESIGN.md §Border Radius: cards/panels = rounded-xl (12px). 16px padding.
    card: {
      borderRadius: radius.lg,
      borderWidth: 1,
      borderColor: colors.border,
      backgroundColor: colors.card,
      padding: spacing.xxl,
      gap: spacing.md,
    },
    cardTitle: {
      ...typeScale.title,
      color: colors.textPrimary,
    },
    // DESIGN.md §DetailRow: flex items-center justify-between px-4 py-2.5 text-[12px].
    // Label = text-muted-foreground; value = font-medium text-foreground.
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
      ...typeScale.label,
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
