import { useMemo } from "react";
import { BlurView } from "expo-blur";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { radius, spacing } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { StatusBadge } from "./StatusBadge";
import type { ChallengeDetail, ChallengeStatus } from "../lib/api/types";

type HistoryCardProps = {
  item: ChallengeDetail;
  onPress?: () => void;
};

function decisionVariant(status: ChallengeStatus): "decisionApproved" | "decisionDenied" | "decisionExpired" {
  if (status === "APPROVED") return "decisionApproved";
  if (status === "DENIED") return "decisionDenied";
  return "decisionExpired";
}

function decisionLabel(status: ChallengeStatus): string {
  if (status === "APPROVED") return "APPROVED";
  if (status === "DENIED") return "DENIED";
  return "EXPIRED";
}

function decisionDescription(status: ChallengeStatus): string {
  if (status === "APPROVED") return "You approved this";
  if (status === "DENIED") return "You denied this";
  return "This request expired";
}

function formatTime(dateStr: string): string {
  const d = new Date(dateStr);
  return d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
}

export function HistoryCard({ item, onPress }: HistoryCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const modeLabel = item.approval_mode === "grant" ? "Grant" : "Per-request";

  const riskVariant = item.risk_level === "high" ? "riskHigh" : item.risk_level === "medium" ? "riskMedium" : "riskLow";

  return (
    <Pressable style={styles.card} onPress={onPress}>
      <View style={styles.chipRow}>
        <StatusBadge variant={decisionVariant(item.status)} label={decisionLabel(item.status)} />
        <StatusBadge variant={riskVariant} label={item.risk_level.toUpperCase()} />
      </View>
      <Text style={styles.title} numberOfLines={1}>
        {item.action} {item.resource}
      </Text>
      <Text style={styles.secondary} numberOfLines={1}>
        {item.title} · {modeLabel}
      </Text>
      <Text style={styles.meta}>
        {formatTime(item.created_at)} · {decisionDescription(item.status)}
      </Text>
    </Pressable>
  );
}

export function HistorySectionHeader({ title }: { title: string }) {
  const { colors, mode } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const inner = <Text style={styles.sectionHeader}>{title}</Text>;

  return (
    <View style={styles.sectionHeaderRow}>
      {mode === "dark" ? (
        <BlurView intensity={60} tint="dark" style={styles.sectionHeaderBlur}>
          {inner}
        </BlurView>
      ) : (
        <View style={styles.sectionHeaderBlur}>
          {inner}
        </View>
      )}
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    card: {
      borderRadius: radius.md,
      backgroundColor: c.cardSoft,
      borderWidth: 1,
      borderColor: c.border,
      padding: spacing.lg,
      gap: 4,
      opacity: 0.8,
    },
    chipRow: {
      flexDirection: "row",
      gap: 6,
    },
    title: {
      flex: 1,
      fontSize: 14,
      fontWeight: "700",
      color: c.textPrimary,
    },
    secondary: {
      fontSize: 13,
      color: c.textSecondary,
    },
    meta: {
      fontSize: 12,
      color: c.textMuted,
    },
    sectionHeaderRow: {
      flexDirection: "row",
      marginBottom: spacing.sm,
      marginTop: spacing.xs,
    },
    sectionHeaderBlur: {
      paddingHorizontal: spacing.sm,
      paddingVertical: spacing.xs,
      borderRadius: 6,
      overflow: "hidden",
      backgroundColor: c.cardSoft,
      borderWidth: 1,
      borderColor: c.borderSoft,
    },
    sectionHeader: {
      fontSize: 11,
      fontWeight: "600",
      color: c.textSecondary,
      textTransform: "uppercase",
      letterSpacing: 0.4,
    },
  });
