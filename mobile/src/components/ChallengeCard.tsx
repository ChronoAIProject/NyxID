import { ServiceIcon } from "./ServiceIcon";
import { getFrontendUrl } from "../lib/env";
import { useMemo } from "react";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";
import { ChevronRight } from "lucide-react-native";
import { TOUCH_TARGET, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { GroupedCell } from "./GroupedCell";
import { SettingsRow } from "./settings/SettingsList";
import type { ChallengeDetail } from "../lib/api/types";

type ChallengeCardProps = {
  challenge: ChallengeDetail;
  grantDurationLabel?: string;
  isMutating?: boolean;
  decisionsDisabled?: boolean;
  /** Position in its inset group, for the rounded corners and separators. */
  first?: boolean;
  last?: boolean;
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

/** Risk tone shared by the request rows and the request detail. */
export function riskTextColor(
  colors: ThemeColors,
  risk: ChallengeDetail["risk_level"],
) {
  return risk === "high"
    ? colors.riskHigh.text
    : risk === "medium"
      ? colors.riskMedium.text
      : risk === "low"
        ? colors.riskLow.text
        : colors.textSecondary;
}

/**
 * Approve | Deny as one split row of tinted text, like the actions of an iOS
 * notification: green approval, red denial, no button chrome.
 */
export function ApprovalDecisionPair({
  disabled,
  divider = true,
  onApprove,
  onDeny,
}: {
  disabled: boolean;
  /** Hairline above the pair when it follows the request content. */
  divider?: boolean;
  onApprove?: () => void;
  onDeny?: () => void;
}) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  return (
    <View style={[styles.pair, divider && styles.pairDivider]}>
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ disabled }}
        disabled={disabled}
        onPress={onApprove}
        style={({ pressed }) => [
          styles.decision,
          pressed && !disabled && styles.pressed,
        ]}
      >
        <Text
          style={[
            styles.decisionLabel,
            { color: disabled ? colors.textMuted : colors.success },
          ]}
        >
          Approve
        </Text>
      </Pressable>
      <View style={styles.pairSeparator} />
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ disabled }}
        disabled={disabled}
        onPress={onDeny}
        style={({ pressed }) => [
          styles.decision,
          pressed && !disabled && styles.pressed,
        ]}
      >
        <Text
          style={[
            styles.decisionLabel,
            { color: disabled ? colors.textMuted : colors.danger },
          ]}
        >
          Deny
        </Text>
      </Pressable>
    </View>
  );
}

/** A pending request: an inset-group row that opens its detail, with Approve | Deny below. */
export function ChallengeCard({
  challenge,
  grantDurationLabel = "30 days",
  isMutating = false,
  decisionsDisabled = false,
  first = true,
  last = true,
  onPress,
  onApprove,
  onDeny,
}: ChallengeCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  // DESIGN.md §Interaction Rules: status labels always Title Case.
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
  // The org label falls back to a generic "Org" when the server omitted the
  // name, so old backends still render something meaningful.
  const context = [
    challenge.from_org_policy === true
      ? challenge.org_name
        ? `Org · ${challenge.org_name}`
        : "Org"
      : undefined,
    modeLabel,
  ].filter(Boolean);
  const showRisk = challenge.risk_level !== "unknown";

  return (
    <GroupedCell first={first} last={last}>
      <Pressable
        style={({ pressed }) => [
          styles.row,
          pressed && onPress && !isMutating && styles.pressed,
        ]}
        onPress={onPress}
        disabled={isMutating}
        accessibilityRole={onPress ? "button" : undefined}
        accessibilityLabel={summary}
        accessibilityHint={
          onPress ? "View approval request details" : undefined
        }
      >
        <View style={styles.tile}>
          <ServiceIcon slug={challenge.service_slug} size="md" />
        </View>
        <View style={styles.body}>
          <Text style={styles.title} numberOfLines={2}>
            {challenge.title}
          </Text>
          <Text style={styles.resource}>{summary}</Text>
          {showRequester ? (
            <Text style={styles.meta}>Requested by {requester}</Text>
          ) : null}
          <Text style={styles.meta}>
            {timeAgo} · {expiryLabel}
          </Text>
          <Text style={styles.meta}>
            {showRisk ? (
              <Text
                style={{ color: riskTextColor(colors, challenge.risk_level) }}
              >
                {`${riskLabel} risk · `}
              </Text>
            ) : null}
            {context.join(" · ")}
          </Text>
          <Text style={styles.meta}>
            {challenge.execution_proposal
              ? "Review the exact scope and duration in the web app."
              : challenge.approval_mode === "grant"
                ? `Approving allows matching requests for ${grantDurationLabel}.`
                : "Approval applies only to this request."}
          </Text>
        </View>
        {onPress ? <ChevronRight size={18} color={colors.textMuted} /> : null}
      </Pressable>
      {challenge.execution_proposal ? (
        <View style={styles.pairDivider}>
          <SettingsRow
            tone="action"
            label="Review in the web app"
            accessibilityRole="link"
            onPress={() =>
              void Linking.openURL(`${getFrontendUrl()}/assistant/approvals`)
            }
          />
        </View>
      ) : (
        <ApprovalDecisionPair
          disabled={blocked}
          onApprove={onApprove}
          onDeny={onDeny}
        />
      )}
    </GroupedCell>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    row: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.lg,
      minHeight: 52,
      paddingHorizontal: spacing.xxl,
      paddingVertical: spacing.md,
    },
    pressed: { backgroundColor: c.navActive },
    tile: {
      width: 30,
      alignSelf: "flex-start",
      alignItems: "center",
      paddingTop: 2,
    },
    body: { flex: 1, minWidth: 0, gap: 2 },
    // DESIGN.md: Activity rows lead with the service name at 15px.
    title: { ...typeScale.title, color: c.textPrimary },
    resource: { ...typeScale.label, color: c.textSecondary },
    meta: { ...typeScale.small, color: c.textMuted },
    pair: { flexDirection: "row", alignItems: "stretch" },
    pairDivider: {
      borderTopWidth: StyleSheet.hairlineWidth,
      borderTopColor: c.borderSoft,
    },
    pairSeparator: {
      width: StyleSheet.hairlineWidth,
      backgroundColor: c.borderSoft,
    },
    decision: {
      flex: 1,
      minHeight: TOUCH_TARGET,
      alignItems: "center",
      justifyContent: "center",
      paddingHorizontal: spacing.md,
    },
    decisionLabel: { ...typeScale.bodyStrong },
  });
