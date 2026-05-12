import { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

type EmptyStatePreset = "pendingEmpty" | "activeEmpty" | "historyEmpty";

type EmptyStateProps = {
  preset: EmptyStatePreset;
};

type PresetConfig = {
  icon: string;
  title: string;
  subtitle: string;
  colorKey: "warning" | "success" | "info";
};

const presetConfigs: Record<EmptyStatePreset, PresetConfig> = {
  pendingEmpty: {
    icon: "!",
    title: "No pending challenges",
    subtitle: "New high-risk requests will appear here.",
    colorKey: "warning",
  },
  activeEmpty: {
    icon: "\u2713",
    title: "No active approvals",
    subtitle: "Approvals appear here after challenge decisions.",
    colorKey: "success",
  },
  historyEmpty: {
    icon: "\u21BA",
    title: "No decision history",
    subtitle: "Past approvals, denials, and expirations appear here.",
    colorKey: "info",
  },
};

function getPresetColors(c: ThemeColors) {
  return {
    warning: { border: c.warningSoft, bg: c.primaryGlow },
    success: { border: c.successSoft, bg: c.primaryGlow },
    info: { border: c.infoSoft, bg: c.primaryGlow },
  } as const;
}

export function EmptyState({ preset }: EmptyStateProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const p = presetConfigs[preset];
  const pc = getPresetColors(colors)[p.colorKey];
  return (
    <View style={styles.container}>
      <View style={[styles.iconWrap, { borderColor: pc.border, backgroundColor: pc.bg }]}>
        <Text style={[styles.icon, { color: colors[p.colorKey] }]}>{p.icon}</Text>
      </View>
      <Text style={styles.title}>{p.title}</Text>
      <Text style={styles.subtitle}>{p.subtitle}</Text>
    </View>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  container: {
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.card,
    padding: spacing.xxl,
    alignItems: "center",
    gap: spacing.sm,
  },
  iconWrap: {
    width: 40,
    height: 40,
    borderRadius: radius.full,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  icon: {
    ...typeScale.bodyStrong,
  },
  title: {
    ...typeScale.title,
    color: c.textPrimary,
    textAlign: "center",
  },
  subtitle: {
    ...typeScale.body,
    color: c.textMuted,
    textAlign: "center",
  },
});
