import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { StatusBadge } from "./StatusBadge";
import type { ApprovalItem } from "../lib/api/types";

type GrantCardProps = {
  grant: ApprovalItem;
  onRevoke?: () => void;
  isMutating?: boolean;
};

function formatDate(dateStr: string): string {
  const d = new Date(dateStr);
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function daysUntilExpiry(expiresAt: string): number {
  return Math.round((new Date(expiresAt).getTime() - Date.now()) / (24 * 60 * 60 * 1000));
}

function formatExpiry(expiresAt: string): string {
  const days = daysUntilExpiry(expiresAt);
  if (days < 0) return "Expired";
  if (days === 0) return "Today";
  if (days === 1) return "Tomorrow";
  return `${days}d left`;
}

export function GrantCard({ grant, onRevoke, isMutating = false }: GrantCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const days = daysUntilExpiry(grant.expires_at);
  const expiryVariant = days < 3 ? "expiryUrgent" : "expiryNormal";
  const expiryLabel = formatExpiry(grant.expires_at);
  const grantedLabel = `Approved ${formatDate(grant.granted_at)}`;
  const expiresLabel = `Expires ${formatDate(grant.expires_at)}`;
  const requesterLabel = grant.requester_label ?? grant.requester_type;
  const showOrgChip = grant.org_scoped === true;
  const orgChipLabel = grant.org_name ? `Org · ${grant.org_name}` : "Org";

  return (
    <View style={styles.card}>
      <View style={styles.header}>
        <Text style={styles.title} numberOfLines={1}>
          {grant.service_name}
        </Text>
        <Pressable
          style={[styles.revokeBtn, isMutating && styles.btnDisabled]}
          onPress={onRevoke}
          disabled={isMutating}
        >
          <Text style={styles.revokeBtnText}>Revoke</Text>
        </Pressable>
      </View>
      {showOrgChip ? (
        <View style={styles.chipRow}>
          <StatusBadge variant="orgChip" label={orgChipLabel} />
        </View>
      ) : null}
      <Text style={styles.secondary} numberOfLines={1}>
        {grant.requester_type} · {requesterLabel}
      </Text>
      <View style={styles.expiryRow}>
        <Text style={styles.meta}>{grantedLabel} · {expiresLabel}</Text>
        <StatusBadge variant={expiryVariant} label={expiryLabel} />
      </View>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    card: {
      borderRadius: radius.lg,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: c.border,
      padding: spacing.lg,
      gap: spacing.sm,
    },
    header: {
      flexDirection: "row",
      justifyContent: "space-between",
      alignItems: "center",
      gap: spacing.sm,
    },
    chipRow: {
      flexDirection: "row",
      gap: spacing.xs,
    },
    title: {
      flex: 1,
      ...typeScale.title,
      color: c.textPrimary,
    },
    secondary: {
      ...typeScale.body,
      color: c.textSecondary,
    },
    expiryRow: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      gap: spacing.sm,
    },
    meta: {
      ...typeScale.small,
      color: c.textMuted,
      flex: 1,
    },
    revokeBtn: {
      paddingHorizontal: spacing.xxl,
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.danger,
      backgroundColor: c.dangerSoftBg,
      alignItems: "center",
      justifyContent: "center",
    },
    revokeBtnText: {
      ...typeScale.label,
      color: c.danger,
    },
    btnDisabled: {
      opacity: 0.5,
    },
  });
