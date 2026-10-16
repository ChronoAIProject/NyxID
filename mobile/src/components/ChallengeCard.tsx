import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import {
  TOUCH_TARGET,
  radius,
  spacing,
  typeScale,
} from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { StatusBadge } from "./StatusBadge";
import type { ChallengeDetail } from "../lib/api/types";

type ChallengeCardProps = {
  challenge: ChallengeDetail;
  grantDurationLabel?: string;
  isMutating?: boolean;
  decisionsDisabled?: boolean;
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
  decisionsDisabled = false,
  onPress,
  onApprove,
  onDeny,
}: ChallengeCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const riskVariant =
    challenge.risk_level === "high"
      ? "riskHigh"
      : challenge.risk_level === "medium"
        ? "riskMedium"
        : challenge.risk_level === "low"
          ? "riskLow"
          : "orgChip";
  // DESIGN.md §Interaction Rules: status badges always Title Case.
  const riskLabel =
    challenge.risk_level.charAt(0).toUpperCase() +
    challenge.risk_level.slice(1);
  const summary =
    challenge.summary.trim() ||
    `${challenge.action} ${challenge.resource}`.trim();
  const requester = challenge.request_context.requester.trim();
  const showRequester =
    requester && requester !== "Unspecified" && requester !== "Unknown";
  const modeLabel =
    challenge.approval_mode === "grant" ? "Grant mode" : "Per-request";
  const timeAgo = formatTimeAgo(challenge.created_at);
  const expiresAt = Date.parse(challenge.expires_at);
  const remainingMs = expiresAt - Date.now();
  const expired = !Number.isFinite(expiresAt) || remainingMs <= 0;
  const expiryLabel = !Number.isFinite(expiresAt)
    ? "Expiry unavailable — refresh before deciding"
    : remainingMs <= 0
      ? "Expired"
      : `Expires in ${Math.max(1, Math.ceil(remainingMs / 60_000))} min`;
  const blocked =
    decisionsDisabled ||
    isMutating ||
    expired ||
    challenge.status !== "PENDING";
  // Render an org chip when the backend marks the request as
  // created under an org approval policy. The label falls back to
  // a generic "Org" when the server omitted the name, so old
  // backends still render something meaningful.
  const showOrgChip = challenge.from_org_policy === true;
  const orgChipLabel = challenge.org_name
    ? `Org · ${challenge.org_name}`
    : "Org";

  return (
    <Pressable
      style={styles.card}
      onPress={onPress}
      disabled={isMutating}
      accessibilityRole={onPress ? "button" : undefined}
      accessibilityLabel={summary}
      accessibilityHint={onPress ? "View approval request details" : undefined}
    >
      <View style={styles.chipRow}>
        {showOrgChip ? (
          <StatusBadge variant="orgChip" label={orgChipLabel} />
        ) : null}
        {challenge.risk_level !== "unknown" ? (
          <StatusBadge variant={riskVariant} label={`${riskLabel} risk`} />
        ) : null}
        <StatusBadge variant="modeChip" label={modeLabel} />
      </View>
      <Text style={styles.title}>{summary}</Text>
      <Text style={styles.resource}>{challenge.title}</Text>
      {showRequester ? (
        <Text style={styles.meta}>Requested by {requester}</Text>
      ) : null}
      <Text style={styles.meta}>
        {timeAgo} · {expiryLabel}
      </Text>
      <Text style={styles.meta}>
        {challenge.approval_mode === "grant"
          ? `Approving allows matching requests for ${grantDurationLabel}.`
          : "Approval applies only to this request."}
      </Text>
      <View style={styles.actions}>
        <Pressable
          style={[styles.approveBtn, blocked && styles.btnDisabled]}
          onPress={onApprove}
          disabled={blocked}
          accessibilityRole="button"
        >
          <Text style={styles.approveBtnText}>Approve</Text>
        </Pressable>
        <Pressable
          style={[styles.denyBtn, blocked && styles.btnDisabled]}
          onPress={onDeny}
          disabled={blocked}
          accessibilityRole="button"
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
      flexWrap: "wrap",
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
