import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { StatusBadge } from "./StatusBadge";
import type { ChallengeDetail } from "../lib/api/types";

type ChallengeCardProps = {
  challenge: ChallengeDetail;
  grantDurationLabel?: string;
  isMutating?: boolean;
  onPress?: () => void;
  onApprove?: () => void;
  onDeny?: () => void;
};

function formatTimeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export function ChallengeCard({
  challenge,
  grantDurationLabel = "30 days",
  isMutating = false,
  onPress,
  onApprove,
  onDeny,
}: ChallengeCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const riskVariant = challenge.risk_level === "high" ? "riskHigh" : challenge.risk_level === "medium" ? "riskMedium" : "riskLow";
  // DESIGN.md §Interaction Rules: status badges always Title Case.
  const riskLabel = challenge.risk_level.charAt(0).toUpperCase() + challenge.risk_level.slice(1);
  const modeLabel = challenge.approval_mode === "grant" ? "Grant mode" : "Per-request";
  const durationLabel = challenge.approval_mode === "grant" ? ` · ${grantDurationLabel}` : "";
  const timeAgo = formatTimeAgo(challenge.created_at);
  // Render an org chip when the backend marks the request as
  // created under an org approval policy. The label falls back to
  // a generic "Org" when the server omitted the name, so old
  // backends still render something meaningful.
  const showOrgChip = challenge.from_org_policy === true;
  const orgChipLabel = challenge.org_name
    ? `Org · ${challenge.org_name}`
    : "Org";

  return (
    <Pressable style={styles.card} onPress={onPress} disabled={isMutating}>
      <View style={styles.chipRow}>
        {showOrgChip ? (
          <StatusBadge variant="orgChip" label={orgChipLabel} />
        ) : null}
        <StatusBadge variant={riskVariant} label={riskLabel} />
        <StatusBadge variant="modeChip" label={modeLabel} />
      </View>
      <Text style={styles.title} numberOfLines={1}>
        {challenge.action} {challenge.resource}
      </Text>
      <Text style={styles.resource} numberOfLines={1}>
        {challenge.title}
      </Text>
      <Text style={styles.meta}>
        {durationLabel ? grantDurationLabel : ""}{durationLabel ? " · " : ""}{timeAgo}
      </Text>
      <View style={styles.actions}>
        <Pressable
          style={[styles.approveBtn, isMutating && styles.btnDisabled]}
          onPress={onApprove}
          disabled={isMutating}
        >
          <Text style={styles.approveBtnText}>Approve</Text>
        </Pressable>
        <Pressable
          style={[styles.denyBtn, isMutating && styles.btnDisabled]}
          onPress={onDeny}
          disabled={isMutating}
        >
          <Text style={styles.denyBtnText}>Deny</Text>
        </Pressable>
      </View>
    </Pressable>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    card: {
      // DESIGN.md §Border Radius: cards = rounded-xl (12px); 50%-opacity border on chrome.
      borderRadius: radius.lg,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: c.borderSoft,
      // Pending approval gets a purple left-border accent (mobile-only affordance,
      // analogous to the desktop active-state purple identity rule).
      borderLeftWidth: 2,
      borderLeftColor: c.primary,
      padding: spacing.xxl,
      gap: spacing.xs,
    },
    // DESIGN.md §List page → mobile-card anatomy: primary text text-[13px] font-semibold.
    title: {
      ...typeScale.bodyStrong,
      color: c.textPrimary,
    },
    chipRow: {
      flexDirection: "row",
      gap: spacing.xs,
    },
    resource: {
      ...typeScale.label,
      color: c.textSecondary,
    },
    // Mobile-card metadata rows: text-[11px].
    meta: {
      ...typeScale.small,
      color: c.textMuted,
    },
    actions: {
      flexDirection: "row",
      gap: spacing.sm,
      marginTop: spacing.sm,
    },
    // DESIGN.md §Usage Rules: Approve = success green tint, Deny = error red tint.
    // Purple NOT used on action buttons.
    approveBtn: {
      flex: 1,
      minHeight: TOUCH_TARGET,
      paddingHorizontal: spacing.md,
      borderRadius: radius.md,
      backgroundColor: c.successSoft,
      borderWidth: 1,
      borderColor: c.success,
      alignItems: "center",
      justifyContent: "center",
    },
    approveBtnText: {
      ...typeScale.label,
      color: c.success,
    },
    denyBtn: {
      flex: 1,
      minHeight: TOUCH_TARGET,
      paddingHorizontal: spacing.md,
      borderRadius: radius.md,
      backgroundColor: c.dangerSoftBg,
      borderWidth: 1,
      borderColor: c.danger,
      alignItems: "center",
      justifyContent: "center",
    },
    denyBtnText: {
      ...typeScale.label,
      color: c.danger,
    },
    btnDisabled: {
      opacity: 0.5,
    },
  });
