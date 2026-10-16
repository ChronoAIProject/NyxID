import { useMemo } from "react";
import { BlurView } from "expo-blur";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { StatusBadge } from "./StatusBadge";
import type { ChallengeDetail, ChallengeStatus } from "../lib/api/types";

type HistoryCardProps = {
  item: ChallengeDetail;
  onPress?: () => void;
};

function decisionVariant(
  status: ChallengeStatus,
): "decisionApproved" | "decisionDenied" | "decisionExpired" {
  if (status === "APPROVED") return "decisionApproved";
  if (status === "DENIED") return "decisionDenied";
  return "decisionExpired";
}

// DESIGN.md §Interaction Rules: status badges always Title Case.
function decisionLabel(status: ChallengeStatus): string {
  if (status === "APPROVED") return "Approved";
  if (status === "DENIED") return "Denied";
  if (status === "PENDING") return "Pending";
  return "Expired";
}

function channelLabel(channel?: string | null): string | undefined {
  if (!channel?.trim()) return undefined;
  if (channel === "web") return "Web";
  if (channel === "telegram") return "Telegram";
  if (channel === "mobile" || channel === "push") return "Mobile";
  return channel;
}

function formatTime(dateStr: string): string | undefined {
  const d = new Date(dateStr);
  if (!Number.isFinite(d.getTime())) return undefined;
  return d.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function recordedRequester(item: ChallengeDetail): string | undefined {
  const requester = item.request_context.requester.trim();
  if (requester && requester !== "Unspecified" && requester !== "Unknown")
    return requester;
  const client = item.request_context.client.trim();
  return client && client !== "Unknown" && client !== "Unspecified"
    ? client
    : undefined;
}

export function HistoryCard({ item, onPress }: HistoryCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const summary =
    item.summary.trim() || `${item.action} ${item.resource}`.trim();
  const channel = channelLabel(item.decision_channel);
  const requester = recordedRequester(item);
  const decidedAt = item.decided_at ? formatTime(item.decided_at) : undefined;
  const requestedAt = formatTime(item.created_at);
  const riskVariant =
    item.risk_level === "high"
      ? "riskHigh"
      : item.risk_level === "medium"
        ? "riskMedium"
        : "riskLow";
  const riskLabel =
    item.risk_level.charAt(0).toUpperCase() + item.risk_level.slice(1);

  return (
    <Pressable
      style={styles.card}
      onPress={onPress}
      accessibilityRole={onPress ? "button" : undefined}
      accessibilityLabel={`${decisionLabel(item.status)}: ${summary}`}
      accessibilityHint={onPress ? "View approval request details" : undefined}
    >
      <View style={styles.chipRow}>
        <StatusBadge
          variant={decisionVariant(item.status)}
          label={decisionLabel(item.status)}
        />
        {item.risk_level !== "unknown" ? (
          <StatusBadge variant={riskVariant} label={`${riskLabel} risk`} />
        ) : null}
      </View>
      <Text style={styles.title}>{summary}</Text>
      <Text style={styles.secondary}>
        {item.title}
        {channel ? ` · via ${channel}` : ""}
      </Text>
      {requester ? (
        <Text style={styles.meta}>Requested by {requester}</Text>
      ) : null}
      <Text style={styles.meta}>
        {decidedAt
          ? `Decided ${decidedAt}`
          : requestedAt
            ? `Requested ${requestedAt}`
            : "Time not recorded"}
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
        <View style={styles.sectionHeaderBlur}>{inner}</View>
      )}
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    card: {
      // DESIGN.md §Border Radius: cards = rounded-xl (12px); 50%-opacity border.
      borderRadius: radius.lg,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: c.borderSoft,
      padding: spacing.xxl,
      gap: spacing.xs,
    },
    chipRow: {
      flexDirection: "row",
      flexWrap: "wrap",
      gap: spacing.xs,
    },
    title: {
      // DESIGN.md mobile-card primary text: text-[13px] font-semibold.
      ...typeScale.bodyStrong,
      color: c.textPrimary,
    },
    secondary: {
      ...typeScale.label,
      color: c.textSecondary,
    },
    meta: {
      ...typeScale.small,
      color: c.textMuted,
    },
    sectionHeaderRow: {
      flexDirection: "row",
      marginBottom: spacing.sm,
      marginTop: spacing.xs,
    },
    sectionHeaderBlur: {
      paddingHorizontal: spacing.md,
      paddingVertical: spacing.xs,
      borderRadius: radius.pill,
      overflow: "hidden",
      backgroundColor: c.cardSoft,
      borderWidth: 1,
      borderColor: c.borderSoft,
    },
    sectionHeader: {
      ...typeScale.overline,
      color: c.textSecondary,
    },
  });
