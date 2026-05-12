import { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

type SectionBadgeProps = {
  label: string;
  tone: "success" | "warning" | "info";
};

function makePalette(c: ThemeColors) {
  return {
    success: { color: c.success, border: c.successSoft },
    warning: { color: c.warning, border: c.warningSoft },
    info: { color: c.info, border: c.infoSoft },
  } as const;
}

export function SectionBadge({ label, tone }: SectionBadgeProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const palette = useMemo(() => makePalette(colors), [colors]);

  return (
    <View style={[styles.wrap, { borderColor: palette[tone].border }]}>
      <Text style={[styles.text, { color: palette[tone].color }]}>{label}</Text>
    </View>
  );
}

const createStyles = (_c: ThemeColors) =>
  StyleSheet.create({
    wrap: {
      borderWidth: 1,
      alignSelf: "flex-start",
      // DESIGN.md §Layout: 100px (badges, pills) — pill-shaped
      borderRadius: radius.pill,
      paddingHorizontal: spacing.md,
      paddingVertical: spacing.xs,
    },
    text: {
      ...typeScale.overline,
    },
  });
