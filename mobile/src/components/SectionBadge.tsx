import { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors, ToneTriple } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

type SectionBadgeProps = {
  label: string;
  tone: "success" | "warning" | "info";
};

function pickTone(c: ThemeColors, tone: SectionBadgeProps["tone"]): ToneTriple {
  if (tone === "success") return c.successTone;
  if (tone === "warning") return c.warningTone;
  return c.infoTone;
}

export function SectionBadge({ label, tone }: SectionBadgeProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const t = pickTone(colors, tone);

  return (
    <View style={[styles.wrap, { backgroundColor: t.bg, borderColor: t.border }]}>
      <Text style={[styles.text, { color: t.text }]}>{label}</Text>
    </View>
  );
}

const createStyles = (_c: ThemeColors) =>
  StyleSheet.create({
    wrap: {
      borderWidth: 1,
      alignSelf: "flex-start",
      // DESIGN.md §Badges: rounded-md (6px), px-2 py-0.5, text-[10px] font-medium.
      borderRadius: radius.sm,
      paddingHorizontal: spacing.sm,
      paddingVertical: 2,
    },
    text: {
      ...typeScale.badge,
    },
  });
