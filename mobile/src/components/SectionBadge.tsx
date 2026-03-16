import { StyleSheet, Text, View } from "react-native";
import { mobileTheme } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

type SectionBadgeProps = {
  label: string;
  tone: "success" | "warning" | "info";
};

const palette = {
  success: { color: mobileTheme.success, border: "#34D39940" },
  warning: { color: mobileTheme.warning, border: "#F59E0B40" },
  info: { color: mobileTheme.info, border: "#60A5FA40" },
} as const;

export function SectionBadge({ label, tone }: SectionBadgeProps) {
  return (
    <View style={[styles.wrap, { borderColor: palette[tone].border }]}>
      <Text style={[styles.text, { color: palette[tone].color }]}>{label}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    borderWidth: 1,
    alignSelf: "flex-start",
    borderRadius: radius.md,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.xs,
  },
  text: {
    ...typeScale.overline,
  },
});
