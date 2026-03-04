import { StyleSheet } from "react-native";
import { mobileTheme } from "./mobileTheme";
import { radius, spacing, typeScale } from "./designTokens";

export const flowStyles = StyleSheet.create({
  content: {
    flex: 1,
  },
  scrollContent: {
    paddingTop: spacing.sm,
    gap: spacing.xl,
    paddingBottom: spacing.xl,
  },
  title: {
    ...typeScale.h1,
    color: mobileTheme.textPrimary,
  },
  subtitle: {
    ...typeScale.body,
    color: mobileTheme.textSecondary,
  },
  card: {
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: mobileTheme.borderSoft,
    backgroundColor: mobileTheme.card,
    padding: spacing.xl,
    gap: spacing.md,
  },
  cardTitle: {
    ...typeScale.title,
    color: mobileTheme.textPrimary,
  },
  row: {
    borderBottomWidth: 1,
    borderBottomColor: mobileTheme.borderSoft,
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
    color: "#9B95B0",
  },
  rowValue: {
    ...typeScale.bodyStrong,
    color: mobileTheme.textPrimary,
    flexShrink: 1,
    textAlign: "right",
  },
  actionWrap: {
    gap: spacing.md,
  },
});
